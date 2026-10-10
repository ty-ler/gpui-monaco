//! What the cursor logic needs to know about the language being edited.
//!
//! Ports, from Monaco's `src/vs/editor/common/languages/` (MIT):
//! `languageConfigurationRegistry.ts` (`ResolvedLanguageConfiguration`:
//! which pairs auto-close and surround when the configuration leaves them
//! out, `getIndentationAtPosition`), `languageConfiguration.ts`
//! (`StandardAutoClosingPairConditional`, `AutoClosingPairs`),
//! `supports/characterPair.ts` (the default `autoCloseBefore` sets),
//! `supports/onEnter.ts`, `supports/indentRules.ts`,
//! `supports/electricCharacter.ts`, `supports/indentationLineProcessor.ts`,
//! `enterAction.ts` and `autoIndent.ts`, plus the
//! `_getShouldAutoClose` part of `src/vs/editor/common/cursorCommon.ts`.
//!
//! Monaco reads token types from its tokenizer; here they come from
//! [`LanguageContext::token_type_at`]. Embedded languages (a different
//! language configuration for part of a line) are not supported: the whole
//! text uses one configuration.

use std::ops::Range;

use syntax::{IndentAction, LanguageConfiguration, NotIn, Pattern};

use super::typing::ShiftCommand;
use super::{
    AutoClosingStrategy, AutoIndent, CursorConfig, CursorModel, Position, line_units, strings,
};

/// The kind of token a character belongs to, as far as editing cares
/// (Monaco `StandardTokenType`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StandardTokenType {
    /// Code, or anything not covered below.
    #[default]
    Other,
    /// Inside a comment.
    Comment,
    /// Inside a string literal.
    String,
    /// Inside a regular expression literal.
    RegEx,
}

impl StandardTokenType {
    /// Monaco `ignoreBracketsInToken`.
    pub(crate) fn ignores_brackets(self) -> bool {
        self != StandardTokenType::Other
    }
}

fn all_other(_: Position) -> StandardTokenType {
    StandardTokenType::Other
}

/// The language configuration and token information the `*_with_language`
/// editing functions use.
pub struct LanguageContext<'a> {
    /// The language's `language-configuration.json`.
    pub config: &'a LanguageConfiguration,
    /// The token type of the character just before `position` (so the
    /// character at column `c` is `token_type_at(Position::new(row, c + 1))`).
    /// Used for `notIn` of auto-closing pairs, to ignore brackets in strings
    /// and comments, and for electric characters. Return
    /// [`StandardTokenType::Other`] when unknown.
    pub token_type_at: &'a dyn Fn(Position) -> StandardTokenType,
    /// The token type a character typed at `position` would get (Monaco
    /// `getTokenTypeIfInsertingCharacter`), used to avoid auto-closing a
    /// quote that closes a string. When `None` it is guessed from the
    /// neighbouring characters' token types.
    pub token_type_if_inserting: Option<&'a dyn Fn(Position, char) -> StandardTokenType>,
    /// Monaco `maxTokenizationLineLength`: longer lines are one plain
    /// token, so they're processed without asking for token types.
    pub max_tokenization_line_length: u32,
}

impl<'a> LanguageContext<'a> {
    /// A context with `token_type_at` and no
    /// [`Self::token_type_if_inserting`].
    pub fn new(
        config: &'a LanguageConfiguration,
        token_type_at: &'a dyn Fn(Position) -> StandardTokenType,
    ) -> Self {
        Self {
            config,
            token_type_at,
            token_type_if_inserting: None,
            max_tokenization_line_length: u32::MAX,
        }
    }

    /// Sets [`Self::max_tokenization_line_length`].
    pub fn with_max_tokenization_line_length(mut self, length: u32) -> Self {
        self.max_tokenization_line_length = length;
        self
    }

    /// A context without token information: every character is
    /// [`StandardTokenType::Other`].
    pub fn untokenized(config: &'a LanguageConfiguration) -> Self {
        Self::new(config, &all_other)
    }

    /// Sets [`Self::token_type_if_inserting`].
    pub fn with_token_type_if_inserting(
        mut self,
        f: &'a dyn Fn(Position, char) -> StandardTokenType,
    ) -> Self {
        self.token_type_if_inserting = Some(f);
        self
    }
}

/// Monaco `CharacterPairSupport.DEFAULT_AUTOCLOSE_BEFORE_LANGUAGE_DEFINED_QUOTES`.
const DEFAULT_AUTOCLOSE_BEFORE_QUOTES: &str = ";:.,=}])> \n\t";
/// Monaco `CharacterPairSupport.DEFAULT_AUTOCLOSE_BEFORE_LANGUAGE_DEFINED_BRACKETS`.
const DEFAULT_AUTOCLOSE_BEFORE_BRACKETS: &str = "'\"`;:.,=}])> \n\t";

/// Monaco `isQuote`.
pub(crate) fn is_quote(ch: &str) -> bool {
    matches!(ch, "'" | "\"" | "`")
}

/// Monaco `StandardAutoClosingPairConditional`.
#[derive(Clone, Debug)]
pub(crate) struct AutoClosingPair {
    pub open: String,
    pub close: String,
    in_string: bool,
    in_comment: bool,
    in_regex: bool,
}

impl AutoClosingPair {
    fn new(open: &str, close: &str, not_in: &[NotIn]) -> Self {
        Self {
            open: open.to_string(),
            close: close.to_string(),
            in_string: !not_in.contains(&NotIn::String),
            in_comment: !not_in.contains(&NotIn::Comment),
            in_regex: !not_in.contains(&NotIn::Regex),
        }
    }

    pub fn open_len(&self) -> u32 {
        strings::utf16_len(&self.open)
    }

    pub fn close_len(&self) -> u32 {
        strings::utf16_len(&self.close)
    }

    pub fn is_ok(&self, token: StandardTokenType) -> bool {
        match token {
            StandardTokenType::Other => true,
            StandardTokenType::Comment => self.in_comment,
            StandardTokenType::String => self.in_string,
            StandardTokenType::RegEx => self.in_regex,
        }
    }

    /// Monaco `findNeutralCharacter`: a character in `[0-9a-zA-Z]` that
    /// appears in neither the opening nor the closing text.
    pub fn find_neutral_character(&self) -> Option<char> {
        ('0'..='9')
            .chain('a'..='z')
            .chain('A'..='Z')
            .find(|&c| !self.open.contains(c) && !self.close.contains(c))
    }
}

fn last_char(s: &str) -> Option<char> {
    s.chars().next_back()
}

fn first_char(s: &str) -> Option<char> {
    s.chars().next()
}

/// The language configuration resolved the way Monaco's
/// `ResolvedLanguageConfiguration` and `CursorConfiguration` do.
pub(crate) struct LanguageSupport<'a> {
    pub ctx: &'a LanguageContext<'a>,
    pub auto_closing_pairs: Vec<AutoClosingPair>,
    /// `(open, close)`.
    pub surrounding_pairs: Vec<(String, String)>,
    auto_close_before_quotes: String,
    auto_close_before_brackets: String,
    /// Monaco `OnEnterSupport` brackets as `(open regex, close regex)`;
    /// `None` when the language has no onEnter support at all.
    on_enter_brackets: Option<Vec<(Pattern, Pattern)>>,
    /// The bracket texts removed from strings and comments before matching
    /// indentation rules (Monaco `bracketsNew.getBracketRegExp`).
    bracket_regex: Option<Pattern>,
    pub block_comment_start: Option<String>,
}

/// Monaco `strings.escapeRegExpCharacters`.
fn escape_regex(s: &str) -> String {
    let mut result = String::new();
    for c in s.chars() {
        if "\\{}*+?|^$.[]()".contains(c) {
            result.push('\\');
        }
        result.push(c);
    }
    result
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

impl<'a> LanguageSupport<'a> {
    pub fn new(ctx: &'a LanguageContext<'a>) -> Self {
        let config = ctx.config;
        let auto_closing_pairs: Vec<AutoClosingPair> = if !config.auto_closing_pairs.is_empty() {
            config
                .auto_closing_pairs
                .iter()
                .map(|p| AutoClosingPair::new(&p.open, &p.close, &p.not_in))
                .collect()
        } else {
            config
                .brackets
                .iter()
                .map(|(open, close)| AutoClosingPair::new(open, close, &[]))
                .collect()
        };
        let surrounding_pairs = if !config.surrounding_pairs.is_empty() {
            config
                .surrounding_pairs
                .iter()
                .map(|p| (p.open.clone(), p.close.clone()))
                .collect()
        } else {
            auto_closing_pairs
                .iter()
                .map(|p| (p.open.clone(), p.close.clone()))
                .collect()
        };
        let has_on_enter = !config.brackets.is_empty()
            || config.indentation_rules.is_some()
            || !config.on_enter_rules.is_empty();
        let on_enter_brackets = has_on_enter.then(|| {
            let brackets: Vec<(String, String)> = if config.brackets.is_empty() {
                vec![
                    ("(".into(), ")".into()),
                    ("{".into(), "}".into()),
                    ("[".into(), "]".into()),
                ]
            } else {
                config.brackets.clone()
            };
            brackets
                .iter()
                .filter_map(|(open, close)| {
                    // Monaco `_createOpenBracketRegExp`/`_createCloseBracketRegExp`.
                    let mut open_src = escape_regex(open);
                    if first_char(open).is_some_and(is_word_char) {
                        open_src.insert_str(0, "\\b");
                    }
                    open_src.push_str("\\s*$");
                    let mut close_src = escape_regex(close);
                    if last_char(close).is_some_and(is_word_char) {
                        close_src.push_str("\\b");
                    }
                    close_src.insert_str(0, "^\\s*");
                    let open_re = Pattern::new(open_src, "");
                    let close_re = Pattern::new(close_src, "");
                    (open_re.regex().is_some() && close_re.regex().is_some())
                        .then_some((open_re, close_re))
                })
                .collect()
        });
        let bracket_pieces: Vec<String> = config
            .brackets
            .iter()
            .filter(|(open, close)| !open.is_empty() && !close.is_empty())
            .flat_map(|(open, close)| [open, close])
            .map(|piece| {
                // Monaco `prepareBracketForRegExp`.
                let wordy = piece.chars().all(|c| is_word_char(c) || c == ' ');
                let escaped = escape_regex(piece);
                if wordy {
                    format!("\\b{escaped}\\b")
                } else {
                    escaped
                }
            })
            .collect();
        let bracket_regex = (!bracket_pieces.is_empty())
            .then(|| Pattern::new(format!("({})", bracket_pieces.join(")|(")), "i"));
        Self {
            ctx,
            auto_closing_pairs,
            surrounding_pairs,
            auto_close_before_quotes: config
                .auto_close_before
                .clone()
                .unwrap_or_else(|| DEFAULT_AUTOCLOSE_BEFORE_QUOTES.to_string()),
            auto_close_before_brackets: config
                .auto_close_before
                .clone()
                .unwrap_or_else(|| DEFAULT_AUTOCLOSE_BEFORE_BRACKETS.to_string()),
            on_enter_brackets,
            bracket_regex,
            block_comment_start: config
                .comments
                .block_comment
                .as_ref()
                .map(|(start, _)| start.clone()),
        }
    }

    pub fn config(&self) -> &'a LanguageConfiguration {
        self.ctx.config
    }

    // --- AutoClosingPairs lookups (keys are single characters)

    /// Monaco `autoClosingPairsOpenByEnd.get(ch)`.
    pub fn pairs_open_by_end(&self, ch: &str) -> impl Iterator<Item = &AutoClosingPair> {
        let mut chars = ch.chars();
        let key = chars.next().filter(|_| chars.next().is_none());
        self.auto_closing_pairs
            .iter()
            .filter(move |p| key.is_some() && last_char(&p.open) == key)
    }

    /// Monaco `autoClosingPairsOpenByStart.get(c)`.
    pub fn pairs_open_by_start(&self, c: char) -> impl Iterator<Item = &AutoClosingPair> {
        self.auto_closing_pairs
            .iter()
            .filter(move |p| first_char(&p.open) == Some(c))
    }

    /// Monaco `autoClosingPairsCloseByStart.get(c)`.
    pub fn pairs_close_by_start(&self, c: char) -> impl Iterator<Item = &AutoClosingPair> {
        self.auto_closing_pairs
            .iter()
            .filter(move |p| first_char(&p.close) == Some(c))
    }

    /// Monaco `autoClosingPairsCloseByEnd.get(c)`.
    pub fn pairs_close_by_end(&self, c: char) -> impl Iterator<Item = &AutoClosingPair> {
        self.auto_closing_pairs
            .iter()
            .filter(move |p| last_char(&p.close) == Some(c))
    }

    /// Monaco `autoClosingPairsCloseSingleChar.has(ch)`.
    pub fn has_single_char_close(&self, ch: &str) -> bool {
        self.auto_closing_pairs
            .iter()
            .any(|p| p.open_len() == 1 && p.close_len() == 1 && p.close == ch)
    }

    /// Monaco `CursorConfiguration._getShouldAutoClose`.
    pub fn should_auto_close_before(
        &self,
        strategy: AutoClosingStrategy,
        for_quotes: bool,
        ch: char,
    ) -> bool {
        match strategy {
            AutoClosingStrategy::BeforeWhitespace => ch == ' ' || ch == '\t',
            AutoClosingStrategy::LanguageDefined => {
                let set = if for_quotes {
                    &self.auto_close_before_quotes
                } else {
                    &self.auto_close_before_brackets
                };
                set.contains(ch)
            }
            AutoClosingStrategy::Always => true,
            AutoClosingStrategy::Never => false,
        }
    }

    pub fn surrounding_close(&self, open: &str) -> Option<&str> {
        self.surrounding_pairs
            .iter()
            .find(|(o, _)| o == open)
            .map(|(_, c)| c.as_str())
    }

    // --- tokens

    /// The token type of the character at UTF-16 `index` of `row`.
    pub fn char_type(&self, row: u32, index: u32) -> StandardTokenType {
        (self.ctx.token_type_at)(Position::new(row, index + 1))
    }

    /// Whether brackets at `p` should be ignored (they are in a string,
    /// comment or regex).
    pub fn is_skippable(&self, p: Position) -> bool {
        self.char_type(p.row, p.column).ignores_brackets()
    }

    /// The token Monaco's `shouldAutoClose` looks at for a cursor at column
    /// `c`: the one containing the character before it (the first token at
    /// the start of the line).
    pub fn token_before(&self, model: &dyn CursorModel, row: u32, c: u32) -> StandardTokenType {
        let len = model.line_max_column(row);
        if len == 0 {
            return StandardTokenType::Other;
        }
        self.char_type(row, c.saturating_sub(1).min(len - 1))
    }

    /// The token at a cursor at column `c` (Monaco `findTokenIndexAtOffset(c)`:
    /// the character after the cursor, or the last one at the end of the
    /// line).
    pub fn token_at(&self, model: &dyn CursorModel, row: u32, c: u32) -> StandardTokenType {
        let len = model.line_max_column(row);
        if len == 0 {
            return StandardTokenType::Other;
        }
        self.char_type(row, c.min(len - 1))
    }

    /// Monaco `getTokenTypeIfInsertingCharacter`.
    pub fn token_type_if_inserting(
        &self,
        model: &dyn CursorModel,
        row: u32,
        c: u32,
        ch: char,
    ) -> StandardTokenType {
        if let Some(f) = self.ctx.token_type_if_inserting {
            return f(Position::new(row, c), ch);
        }
        // Guess: inside a token if both neighbours belong to it.
        let len = model.line_max_column(row);
        let before = (c > 0).then(|| self.char_type(row, c - 1));
        let after = (c < len).then(|| self.char_type(row, c));
        match (before, after) {
            (Some(b), Some(a)) if a == b => a,
            (Some(b), None) => b,
            _ => StandardTokenType::Other,
        }
    }

    /// The text of `row` between UTF-16 columns `from` and `to`, with
    /// brackets removed from strings, comments and regexes (Monaco
    /// `IndentationLineProcessor.getProcessedTokens`).
    pub fn processed(&self, model: &dyn CursorModel, row: u32, from: u32, to: u32) -> String {
        // An untokenized long line is one plain token: no brackets to drop.
        if model.line_max_column(row) >= self.ctx.max_tokenization_line_length {
            return model.line_slice(row, from..to);
        }
        let units = line_units(model, row);
        let to = (to as usize).min(units.len());
        let from = (from as usize).min(to);
        let Some(regex) = self.bracket_regex.as_ref().and_then(|p| p.regex()) else {
            return String::from_utf16_lossy(&units[from..to]);
        };
        let mut result = String::new();
        let mut start = from;
        while start < to {
            let token = self.char_type(row, start as u32);
            let mut end = start + 1;
            while end < to && self.char_type(row, end as u32) == token {
                end += 1;
            }
            let text = String::from_utf16_lossy(&units[start..end]);
            if token.ignores_brackets() {
                result.push_str(&regex.replace_all(&text, ""));
            } else {
                result.push_str(&text);
            }
            start = end;
        }
        result
    }

    pub fn processed_line(&self, model: &dyn CursorModel, row: u32) -> String {
        self.processed(model, row, 0, u32::MAX)
    }

    // --- indentation rules (Monaco `IndentRulesSupport`)

    pub fn has_indentation_rules(&self) -> bool {
        self.config().indentation_rules.is_some()
    }

    pub fn should_increase(&self, text: &str) -> bool {
        self.config()
            .indentation_rules
            .as_ref()
            .is_some_and(|r| r.increase_indent_pattern.is_match(text))
    }

    pub fn should_decrease(&self, text: &str) -> bool {
        self.config()
            .indentation_rules
            .as_ref()
            .is_some_and(|r| r.decrease_indent_pattern.is_match(text))
    }

    pub fn should_indent_next_line(&self, text: &str) -> bool {
        self.config()
            .indentation_rules
            .as_ref()
            .and_then(|r| r.indent_next_line_pattern.as_ref())
            .is_some_and(|p| p.is_match(text))
    }

    pub fn should_ignore(&self, text: &str) -> bool {
        self.config()
            .indentation_rules
            .as_ref()
            .and_then(|r| r.un_indented_line_pattern.as_ref())
            .is_some_and(|p| p.is_match(text))
    }

    /// Monaco `IndentRulesSupport.getIndentMetadata`: which indentation
    /// rules `text` matches, as a bitmask of the [`IndentConsts`] masks.
    pub fn indent_metadata(&self, text: &str) -> u8 {
        let mut ret = 0;
        if self.should_increase(text) {
            ret |= IndentConsts::INCREASE_MASK;
        }
        if self.should_decrease(text) {
            ret |= IndentConsts::DECREASE_MASK;
        }
        if self.should_indent_next_line(text) {
            ret |= IndentConsts::INDENT_NEXTLINE_MASK;
        }
        if self.should_ignore(text) {
            ret |= IndentConsts::UNINDENT_MASK;
        }
        ret
    }

    // --- onEnter (Monaco `OnEnterSupport.onEnter`)

    pub fn on_enter(
        &self,
        auto_indent: AutoIndent,
        previous_line_text: &str,
        before_enter_text: &str,
        after_enter_text: &str,
    ) -> Option<EnterResult> {
        let brackets = self.on_enter_brackets.as_ref()?;
        // (1): regExpRules
        if auto_indent >= AutoIndent::Advanced {
            for rule in &self.config().on_enter_rules {
                let matches = rule.before_text.is_match(before_enter_text)
                    && rule
                        .after_text
                        .as_ref()
                        .is_none_or(|p| p.is_match(after_enter_text))
                    && rule
                        .previous_line_text
                        .as_ref()
                        .is_none_or(|p| p.is_match(previous_line_text));
                if matches {
                    return Some(EnterResult {
                        indent_action: rule.action.indent,
                        append_text: rule.action.append_text.clone(),
                        remove_text: rule.action.remove_text,
                    });
                }
            }
        }
        // (2): Special indent-outdent
        if auto_indent >= AutoIndent::Brackets
            && !before_enter_text.is_empty()
            && !after_enter_text.is_empty()
        {
            for (open, close) in brackets {
                if open.is_match(before_enter_text) && close.is_match(after_enter_text) {
                    return Some(EnterResult {
                        indent_action: IndentAction::IndentOutdent,
                        append_text: None,
                        remove_text: None,
                    });
                }
            }
        }
        // (4): Open bracket based logic
        if auto_indent >= AutoIndent::Brackets && !before_enter_text.is_empty() {
            for (open, _) in brackets {
                if open.is_match(before_enter_text) {
                    return Some(EnterResult {
                        indent_action: IndentAction::Indent,
                        append_text: None,
                        remove_text: None,
                    });
                }
            }
        }
        None
    }
}

/// Monaco `IndentConsts`: the bits of [`LanguageSupport::indent_metadata`].
pub(crate) struct IndentConsts;

impl IndentConsts {
    /// The line matches `increaseIndentPattern`.
    pub const INCREASE_MASK: u8 = 0b0000_0001;
    /// The line matches `decreaseIndentPattern`.
    pub const DECREASE_MASK: u8 = 0b0000_0010;
    /// The line matches `indentNextLinePattern`.
    pub const INDENT_NEXTLINE_MASK: u8 = 0b0000_0100;
    /// The line matches `unIndentedLinePattern`.
    pub const UNINDENT_MASK: u8 = 0b0000_1000;
}

/// Monaco `EnterAction`.
#[derive(Clone, Debug)]
pub(crate) struct EnterResult {
    pub indent_action: IndentAction,
    pub append_text: Option<String>,
    pub remove_text: Option<u32>,
}

/// Monaco `CompleteEnterAction`.
#[derive(Clone, Debug)]
pub(crate) struct CompleteEnterAction {
    pub indent_action: IndentAction,
    pub append_text: String,
    pub indentation: String,
}

// ---------------------------------------------------------------------------
// Indentation helpers shared with the typing operations.

/// Monaco `shiftIndent(config, indentation)`.
pub(crate) fn shift_indent(config: &CursorConfig, indentation: &str) -> String {
    ShiftCommand::shift_indent(
        indentation,
        strings::utf16_len(indentation),
        config.tab_size,
        config.indent_size,
        config.insert_spaces,
        false,
    )
}

/// Monaco `unshiftIndent(config, indentation)`.
pub(crate) fn unshift_indent(config: &CursorConfig, indentation: &str) -> String {
    ShiftCommand::shift_indent(
        indentation,
        strings::utf16_len(indentation),
        config.tab_size,
        config.indent_size,
        config.insert_spaces,
        true,
    )
}

fn leading_whitespace(s: &str) -> String {
    strings::leading_whitespace(s).to_string()
}

/// JavaScript `/^\s*$/`.
pub(crate) fn is_blank(s: &str) -> bool {
    s.chars().all(char::is_whitespace)
}

/// Monaco `getIndentationAtPosition`.
pub(crate) fn indentation_at_position(model: &dyn CursorModel, p: Position) -> String {
    let line = model.line_content(p.row);
    strings::utf16_prefix(strings::leading_whitespace(&line), p.column).to_string()
}

/// The UTF-16 prefix of `s` that is `len - remove` units long.
fn remove_suffix_units(s: &str, remove: u32) -> String {
    let len = strings::utf16_len(s);
    strings::utf16_prefix(s, len.saturating_sub(remove)).to_string()
}

// ---------------------------------------------------------------------------
// enterAction.ts / autoIndent.ts

/// Monaco `IVirtualModel`: the model, optionally with one line replaced.
/// Line numbers are 1-based, as in Monaco.
pub(crate) struct VirtualModel<'a> {
    model: &'a dyn CursorModel,
    lang: &'a LanguageSupport<'a>,
    replaced: Option<(u32, String)>,
    /// Whether `replaced` also stands for the line's processed text (Monaco
    /// `createVirtualModelWithModifiedTokensAtLine`, already processed). When
    /// `false` only the line's content is replaced and the processed text
    /// still comes from the model's tokens, like the virtual model of
    /// `AutoIndentOnPaste`.
    replaced_processed: bool,
}

impl<'a> VirtualModel<'a> {
    /// The model itself.
    pub(crate) fn new(model: &'a dyn CursorModel, lang: &'a LanguageSupport<'a>) -> Self {
        Self {
            model,
            lang,
            replaced: None,
            replaced_processed: false,
        }
    }

    /// The model with the content (not the tokens) of 1-based `line_number`
    /// replaced by `text`.
    pub(crate) fn with_line_content(mut self, line_number: u32, text: String) -> Self {
        self.replaced = Some((line_number, text));
        self.replaced_processed = false;
        self
    }

    fn line(&self, line_number: u32) -> String {
        match &self.replaced {
            Some((n, text)) if *n == line_number => text.clone(),
            _ => self.model.line_content(line_number - 1),
        }
    }

    /// Monaco `ProcessedIndentRulesSupport`'s processed line.
    fn processed(&self, line_number: u32) -> String {
        match &self.replaced {
            Some((n, text)) if *n == line_number && self.replaced_processed => text.clone(),
            _ => self.lang.processed_line(self.model, line_number - 1),
        }
    }

    fn should_increase(&self, line_number: u32) -> bool {
        self.lang.should_increase(&self.processed(line_number))
    }

    fn should_decrease(&self, line_number: u32) -> bool {
        self.lang.should_decrease(&self.processed(line_number))
    }

    fn should_indent_next_line(&self, line_number: u32) -> bool {
        self.lang
            .should_indent_next_line(&self.processed(line_number))
    }

    fn should_ignore(&self, line_number: u32) -> bool {
        self.lang.should_ignore(&self.processed(line_number))
    }
}

/// The result of [`inherit_indent_for_line`].
pub(crate) struct InheritIndent {
    pub indentation: String,
    pub action: Option<IndentAction>,
    /// The 1-based line the indentation was inherited from.
    pub line: Option<u32>,
}

/// Monaco `getPrecedingValidLine`: the nearest line above that is neither
/// blank nor matches `unIndentedLinePattern`, or `None`.
fn preceding_valid_line(vm: &VirtualModel, line_number: u32) -> Option<u32> {
    (1..line_number).rev().find(|&last| {
        let text = vm.line(last);
        !(vm.should_ignore(last) || is_blank(&text))
    })
}

/// Monaco `getInheritIndentForLine` (1-based `line_number`).
fn inherit_indent_for_line(
    auto_indent: AutoIndent,
    vm: &VirtualModel,
    line_number: u32,
    honor_intentional_indent: bool,
) -> Option<InheritIndent> {
    if auto_indent < AutoIndent::Full || !vm.lang.has_indentation_rules() {
        return None;
    }
    let none = |indentation: String, line: Option<u32>| {
        Some(InheritIndent {
            indentation,
            action: None,
            line,
        })
    };
    if line_number <= 1 {
        return none(String::new(), None);
    }
    // Use no indent if this is the first non-blank line.
    for prior in (1..line_number).rev() {
        if !vm.line(prior).is_empty() {
            break;
        }
        if prior == 1 {
            return none(String::new(), None);
        }
    }
    let preceding = preceding_valid_line(vm, line_number)?;
    let lw = |n: u32| leading_whitespace(&vm.line(n));
    if vm.should_increase(preceding) || vm.should_indent_next_line(preceding) {
        return Some(InheritIndent {
            indentation: lw(preceding),
            action: Some(IndentAction::Indent),
            line: Some(preceding),
        });
    }
    if vm.should_decrease(preceding) {
        return none(lw(preceding), Some(preceding));
    }
    // The preceding line neither increases nor decreases the indentation of
    // the lines after it, so take the indentation from the lines above it.
    if preceding == 1 {
        return none(lw(1), Some(1));
    }
    let previous = preceding - 1;
    let previous_text = vm.line(previous);
    let increase_or_decrease =
        vm.lang.should_increase(&previous_text) || vm.lang.should_decrease(&previous_text);
    if !increase_or_decrease && vm.lang.should_indent_next_line(&previous_text) {
        let stop = (1..previous)
            .rev()
            .find(|&i| !vm.should_indent_next_line(i))
            .unwrap_or(0);
        return none(lw(stop + 1), Some(stop + 1));
    }
    if honor_intentional_indent {
        return none(lw(preceding), Some(preceding));
    }
    // Search from the preceding line for one whose indent is not temporary.
    for i in (1..=preceding).rev() {
        if vm.should_increase(i) {
            return Some(InheritIndent {
                indentation: lw(i),
                action: Some(IndentAction::Indent),
                line: Some(i),
            });
        } else if vm.should_indent_next_line(i) {
            // Monaco's inner loop tests line `i` rather than `j`, so it always
            // runs to the top.
            return none(lw(1), Some(1));
        } else if vm.should_decrease(i) {
            return none(lw(i), Some(i));
        }
    }
    none(lw(1), Some(1))
}

/// The processed text before `range` on its first line and after it on its
/// last line (Monaco `IndentationContextProcessor`).
fn processed_around(
    lang: &LanguageSupport,
    model: &dyn CursorModel,
    range: &Range<Position>,
) -> (String, String) {
    let before = lang.processed(model, range.start.row, 0, range.start.column);
    let after_position = range.end;
    let after = lang.processed(model, after_position.row, after_position.column, u32::MAX);
    (before, after)
}

/// Monaco `getEnterAction`.
pub(crate) fn enter_action(
    auto_indent: AutoIndent,
    lang: &LanguageSupport,
    model: &dyn CursorModel,
    range: &Range<Position>,
) -> Option<CompleteEnterAction> {
    let previous_line_text = if range.start.row == 0 {
        String::new()
    } else {
        lang.processed_line(model, range.start.row - 1)
    };
    let (before, after) = processed_around(lang, model, range);
    let result = lang.on_enter(auto_indent, &previous_line_text, &before, &after)?;
    let indent_action = result.indent_action;
    // `\t` stands for one indent; `appendText` and `removeText` adjust it.
    let append_text = match result.append_text.filter(|t| !t.is_empty()) {
        None => match indent_action {
            IndentAction::Indent | IndentAction::IndentOutdent => "\t".to_string(),
            _ => String::new(),
        },
        Some(text) if indent_action == IndentAction::Indent => format!("\t{text}"),
        Some(text) => text,
    };
    let mut indentation = indentation_at_position(model, range.start);
    if let Some(remove) = result.remove_text.filter(|&r| r > 0) {
        indentation = remove_suffix_units(&indentation, remove);
    }
    Some(CompleteEnterAction {
        indent_action,
        append_text,
        indentation,
    })
}

/// Monaco `getIndentForEnter`: the indentation of the line before and after
/// the line break.
pub(crate) fn indent_for_enter(
    config: &CursorConfig,
    lang: &LanguageSupport,
    model: &dyn CursorModel,
    range: &Range<Position>,
) -> Option<(String, String)> {
    if config.auto_indent < AutoIndent::Full || !lang.has_indentation_rules() {
        return None;
    }
    let (before, after) = processed_around(lang, model, range);
    let before_enter_indent = leading_whitespace(&before);
    let vm = VirtualModel {
        model,
        lang,
        replaced: Some((range.start.row + 1, before)),
        replaced_processed: true,
    };
    let Some(after_enter_action) =
        inherit_indent_for_line(config.auto_indent, &vm, range.start.row + 2, true)
    else {
        return Some((before_enter_indent.clone(), before_enter_indent));
    };
    let mut after_enter_indent = after_enter_action.indentation;
    if after_enter_action.action == Some(IndentAction::Indent) {
        after_enter_indent = shift_indent(config, &after_enter_indent);
    }
    if lang.should_decrease(&after) {
        after_enter_indent = unshift_indent(config, &after_enter_indent);
    }
    Some((before_enter_indent, after_enter_indent))
}

/// Monaco `getIndentActionForType`: the indentation the line should get
/// when `ch` is typed over `range`, or `None` to leave it alone.
pub(crate) fn indent_action_for_type(
    config: &CursorConfig,
    lang: &LanguageSupport,
    model: &dyn CursorModel,
    range: &Range<Position>,
    ch: &str,
) -> Option<String> {
    if config.auto_indent < AutoIndent::Full || !lang.has_indentation_rules() {
        return None;
    }
    let (before, after) = processed_around(lang, model, range);
    let text_around_range = format!("{before}{after}");
    let text_around_range_with_character = format!("{before}{ch}{after}");
    let vm = VirtualModel::new(model, lang);

    // If the text already matched decreaseIndentPattern, the user may have
    // adjusted the indentation on purpose; leave it.
    if !lang.should_decrease(&text_around_range)
        && lang.should_decrease(&text_around_range_with_character)
    {
        // After typing `ch` the line matches decreaseIndentPattern.
        let r = inherit_indent_for_line(config.auto_indent, &vm, range.start.row + 1, false)?;
        let mut indentation = r.indentation;
        if r.action != Some(IndentAction::Indent) {
            indentation = unshift_indent(config, &indentation);
        }
        return Some(indentation);
    }

    if range.start == range.end && range.start.row > 0 {
        let previous_line = model.line_content(range.start.row - 1);
        if lang.should_indent_next_line(&previous_line)
            && lang.should_increase(&text_around_range_with_character)
        {
            let inherited =
                inherit_indent_for_line(config.auto_indent, &vm, range.start.row + 1, false);
            if let Some(inherited) = inherited {
                let current_line = model.line_content(range.start.row);
                let actual_current_indentation = strings::leading_whitespace(&current_line);
                let inferred_current_indentation = shift_indent(config, &inherited.indentation);
                // If they differ, the indentation was changed on purpose.
                let inferred_equals_actual =
                    inferred_current_indentation == actual_current_indentation;
                let only_whitespace_around = is_blank(&text_around_range);
                let auto_closing_pair_exists = lang.pairs_open_by_end(ch).next().is_some();
                if inferred_equals_actual && auto_closing_pair_exists && only_whitespace_around {
                    return Some(inherited.indentation);
                }
            }
        }
    }
    None
}

/// Monaco `TabOperation._goodIndentForLine` (0-based `row`).
pub(crate) fn good_indent_for_line(
    config: &CursorConfig,
    lang: &LanguageSupport,
    model: &dyn CursorModel,
    row: u32,
) -> Option<String> {
    let mut action = None;
    let mut indentation = String::new();
    let vm = VirtualModel::new(model, lang);
    if let Some(expected) = inherit_indent_for_line(config.auto_indent, &vm, row + 1, false) {
        action = expected.action;
        indentation = expected.indentation;
    } else if row > 0 {
        let units_blank =
            |r: u32| strings::last_non_whitespace_index(&line_units(model, r), None).is_none();
        let last_row = (0..row).rev().find(|&r| !units_blank(r))?;
        let max_column = model.line_max_column(last_row);
        let at = Position::new(last_row, max_column);
        if let Some(enter) = enter_action(config.auto_indent, lang, model, &(at..at)) {
            indentation = format!("{}{}", enter.indentation, enter.append_text);
        }
    }
    if let Some(action) = action {
        if action == IndentAction::Indent {
            indentation = shift_indent(config, &indentation);
        }
        if action == IndentAction::Outdent {
            indentation = unshift_indent(config, &indentation);
        }
        indentation = config.normalize_indentation(&indentation);
    }
    (!indentation.is_empty()).then_some(indentation)
}

/// Monaco `BracketElectricCharacterSupport.getElectricCharacters`: the last
/// characters of the closing brackets.
pub(crate) fn is_electric_character(lang: &LanguageSupport, ch: &str) -> bool {
    lang.config()
        .brackets
        .iter()
        .filter(|(open, close)| !open.is_empty() && !close.is_empty())
        .any(|(_, close)| last_char(close).is_some_and(|c| c.to_string() == ch))
}

/// Monaco `BracketElectricCharacterSupport.onElectricCharacter`: the closing
/// bracket typing `ch` at `position` completes, if it should be lined up
/// with its opening bracket (it is the first thing on the line).
pub(crate) fn on_electric_character(
    lang: &LanguageSupport,
    model: &dyn CursorModel,
    ch: &str,
    position: Position,
) -> Option<String> {
    let brackets: Vec<(String, String)> = lang
        .config()
        .brackets
        .iter()
        .filter(|(open, close)| !open.is_empty() && !close.is_empty())
        .map(|(open, close)| (open.to_lowercase(), close.to_lowercase()))
        .collect();
    if brackets.is_empty() {
        return None;
    }
    if lang
        .token_at(model, position.row, position.column)
        .ignores_brackets()
    {
        return None;
    }
    let line = model.line_content(position.row);
    let text: Vec<u16> = format!("{}{ch}", strings::utf16_prefix(&line, position.column))
        .to_lowercase()
        .encode_utf16()
        .collect();
    // Monaco `BracketsUtils.findPrevBracketInRange`: the last bracket.
    let pieces: Vec<(Vec<u16>, bool)> = brackets
        .iter()
        .flat_map(|(open, close)| {
            [
                (open.encode_utf16().collect(), true),
                (close.encode_utf16().collect(), false),
            ]
        })
        .collect();
    let (start, bracket, is_open) = (1..=text.len()).rev().find_map(|end| {
        pieces
            .iter()
            .filter(|(piece, _)| text[..end].ends_with(piece))
            .max_by_key(|(piece, _)| piece.len())
            .map(|(piece, is_open)| (end - piece.len(), piece.clone(), *is_open))
    })?;
    if is_open {
        return None;
    }
    let text_before_bracket = String::from_utf16_lossy(&text[..start]);
    if !is_blank(&text_before_bracket) {
        // There is other text on the line before the bracket.
        return None;
    }
    Some(String::from_utf16_lossy(&bracket))
}

/// Monaco `getIndentMetadata` (autoIndent.ts): the indentation rules the
/// content of `row` matches (see [`LanguageSupport::indent_metadata`]), or
/// `None` without indentation rules or for a row outside the model.
pub(crate) fn indent_metadata(
    lang: &LanguageSupport,
    model: &dyn CursorModel,
    row: u32,
) -> Option<u8> {
    if !lang.has_indentation_rules() || row >= model.line_count() {
        return None;
    }
    Some(lang.indent_metadata(&model.line_content(row)))
}

/// Monaco `getGoodIndentForLine` (autoIndent.ts; not to be confused with
/// `TabOperation._goodIndentForLine`, see [`good_indent_for_line`]): the
/// indentation 1-based `line_number` of `vm` should have according to the
/// indentation rules and the onEnter rules of the line it inherits its
/// indentation from, or `None` when the rules don't say.
pub(crate) fn rules_good_indent_for_line(
    config: &CursorConfig,
    vm: &VirtualModel,
    line_number: u32,
) -> Option<String> {
    let lang = vm.lang;
    if config.auto_indent < AutoIndent::Full || !lang.has_indentation_rules() {
        return None;
    }
    let indent = inherit_indent_for_line(config.auto_indent, vm, line_number, true)?;
    if let Some(inherit_line) = indent.line {
        // Apply the enter action as long as there are only whitespace lines
        // between the inherited line and this line.
        let should_apply_enter_rules =
            (inherit_line..line_number.saturating_sub(1)).all(|n| is_blank(&vm.line(n)));
        if should_apply_enter_rules {
            let inherit_text = vm.line(inherit_line);
            if let Some(enter) = lang.on_enter(config.auto_indent, "", &inherit_text, "") {
                let mut indentation = leading_whitespace(&inherit_text);
                if let Some(remove) = enter.remove_text.filter(|&r| r > 0) {
                    indentation = remove_suffix_units(&indentation, remove);
                }
                match enter.indent_action {
                    IndentAction::Indent | IndentAction::IndentOutdent => {
                        indentation = shift_indent(config, &indentation);
                    }
                    IndentAction::Outdent => indentation = unshift_indent(config, &indentation),
                    IndentAction::None => {}
                }
                if vm.should_decrease(line_number) {
                    indentation = unshift_indent(config, &indentation);
                }
                if let Some(append) = &enter.append_text {
                    indentation.push_str(append);
                }
                return Some(leading_whitespace(&indentation));
            }
        }
    }
    let indents = indent.action == Some(IndentAction::Indent);
    Some(if vm.should_decrease(line_number) {
        if indents {
            indent.indentation
        } else {
            unshift_indent(config, &indent.indentation)
        }
    } else if indents {
        shift_indent(config, &indent.indentation)
    } else {
        indent.indentation
    })
}
