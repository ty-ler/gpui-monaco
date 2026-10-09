//! Ports of Monaco's
//! `src/vs/editor/test/common/modes/supports/characterPair.test.ts` (suite
//! `CharacterPairSupport`) and `electricCharacter.test.ts` (suite `Editor
//! Modes - Auto Indentation`) (MIT), run against `LanguageSupport`.
//!
//! Deviation: `brackets is ignored when having autoClosingPairs` expects an
//! explicitly empty `autoClosingPairs` to win over `brackets`; a
//! [`LanguageConfiguration`] cannot tell an empty list from a missing one, so
//! the brackets are used.

use syntax::{LanguageConfiguration, NotIn};

use super::language_configs::{brackets, pair, pair_not_in, surround};
use crate::cursor::language::{LanguageSupport, is_electric_character, on_electric_character};
use crate::cursor::{CursorModel, LanguageContext, Position, StandardTokenType};

type Pairs = Vec<(String, String)>;

/// The auto-closing and surrounding pairs.
fn pairs_of(config: LanguageConfiguration) -> (Pairs, Pairs) {
    let ctx = LanguageContext::untokenized(&config);
    let support = LanguageSupport::new(&ctx);
    (
        support
            .auto_closing_pairs
            .iter()
            .map(|p| (p.open.clone(), p.close.clone()))
            .collect(),
        support.surrounding_pairs.clone(),
    )
}

fn ab() -> Vec<(String, String)> {
    vec![("a".into(), "b".into())]
}

#[test]
fn only_auto_closing_pairs() {
    let config = LanguageConfiguration {
        auto_closing_pairs: vec![pair("a", "b")],
        ..Default::default()
    };
    assert_eq!(pairs_of(config), (ab(), ab()));
}

#[test]
fn only_empty_auto_closing_pairs() {
    assert_eq!(pairs_of(LanguageConfiguration::default()), (vec![], vec![]));
}

#[test]
fn only_brackets() {
    let config = LanguageConfiguration {
        brackets: brackets(&[("a", "b")]),
        ..Default::default()
    };
    assert_eq!(pairs_of(config), (ab(), ab()));
}

#[test]
fn only_surrounding_pairs() {
    let config = LanguageConfiguration {
        surrounding_pairs: vec![surround("a", "b")],
        ..Default::default()
    };
    assert_eq!(pairs_of(config), (vec![], ab()));
}

/// A one-line model with the given tokens, and whether `{` (notIn string
/// and comment, unless `plain`) may auto-close at 1-based `column`.
fn should_auto_close(tokens: &[(&str, StandardTokenType)], column: u32, plain: bool) -> bool {
    let text: String = tokens.iter().map(|(t, _)| *t).collect();
    let types: Vec<StandardTokenType> = tokens
        .iter()
        .flat_map(|(t, ty)| std::iter::repeat_n(*ty, t.encode_utf16().count()))
        .collect();
    let model = text::Rope::from(text.as_str());
    let at = |p: Position| -> StandardTokenType {
        if p.column == 0 {
            return StandardTokenType::Other;
        }
        types
            .get(p.column as usize - 1)
            .copied()
            .unwrap_or_default()
    };
    let config = LanguageConfiguration {
        auto_closing_pairs: vec![if plain {
            pair("{", "}")
        } else {
            pair_not_in("{", "}", &[NotIn::String, NotIn::Comment])
        }],
        ..Default::default()
    };
    let ctx = LanguageContext::new(&config, &at);
    let support = LanguageSupport::new(&ctx);
    let model: &dyn CursorModel = &model;
    support.auto_closing_pairs[0].is_ok(support.token_before(model, 0, column - 1))
}

#[test]
fn should_auto_close_pair_in_empty_line() {
    assert!(should_auto_close(&[], 1, false));
}

#[test]
fn should_auto_close_pair_in_not_interesting_line_1() {
    assert!(should_auto_close(
        &[("do", StandardTokenType::Other)],
        3,
        false
    ));
}

#[test]
fn should_auto_close_pair_in_not_interesting_line_2() {
    assert!(should_auto_close(
        &[("do", StandardTokenType::String)],
        3,
        true
    ));
}

#[test]
fn should_auto_close_pair_in_interesting_line_1() {
    let line = [("\"a\"", StandardTokenType::String)];
    for column in 1..=4 {
        assert!(!should_auto_close(&line, column, false), "{column}");
    }
}

#[test]
fn should_auto_close_pair_in_interesting_line_2() {
    let line = [
        ("x=", StandardTokenType::Other),
        ("\"a\"", StandardTokenType::String),
        (";", StandardTokenType::Other),
    ];
    let expected = [true, true, true, false, false, false, true];
    for (i, expected) in expected.into_iter().enumerate() {
        let column = i as u32 + 1;
        assert_eq!(
            should_auto_close(&line, column, false),
            expected,
            "{column}"
        );
    }
}

#[test]
fn should_auto_close_pair_in_interesting_line_3() {
    let line = [
        (" ", StandardTokenType::Other),
        ("//a", StandardTokenType::Comment),
    ];
    let expected = [true, true, false, false, false];
    for (i, expected) in expected.into_iter().enumerate() {
        let column = i as u32 + 1;
        assert_eq!(
            should_auto_close(&line, column, false),
            expected,
            "{column}"
        );
    }
}

// ---------------------------------------------------------------------------
// electricCharacter.test.ts

fn electric(line: &str, character: &str, offset: u32) -> Option<String> {
    let config = LanguageConfiguration {
        brackets: brackets(&[("{", "}"), ("(", ")")]),
        ..Default::default()
    };
    let ctx = LanguageContext::untokenized(&config);
    let support = LanguageSupport::new(&ctx);
    let model = text::Rope::from(line);
    on_electric_character(&support, &model, character, Position::new(0, offset - 1))
}

#[test]
fn get_electric_characters_uses_all_sources_and_dedups() {
    let config = LanguageConfiguration {
        brackets: brackets(&[("{", "}"), ("(", ")")]),
        ..Default::default()
    };
    let ctx = LanguageContext::untokenized(&config);
    let support = LanguageSupport::new(&ctx);
    let electric: Vec<&str> = ["}", ")", "{", "("]
        .into_iter()
        .filter(|c| is_electric_character(&support, c))
        .collect();
    assert_eq!(electric, ["}", ")"]);
}

#[test]
fn match_open_bracket() {
    assert_eq!(electric("\t{", "\t", 1), None);
    assert_eq!(electric("\t{", "\t", 2), None);
    assert_eq!(electric("\t\t", "{", 3), None);

    assert_eq!(electric("\t}", "\t", 1), None);
    assert_eq!(electric("\t}", "\t", 2), None);
    assert_eq!(electric("\t\t", "}", 3), Some("}".to_string()));
}
