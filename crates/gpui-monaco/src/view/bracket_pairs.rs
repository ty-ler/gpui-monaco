//! Bracket pair nesting for colorization.
//!
//! Follows how Monaco's bracket pair parser
//! (`src/vs/editor/common/model/bracketPairsTextModelPart/bracketPairsTree/parser.ts`)
//! and `collectBrackets` (`bracketPairsTree.ts`) classify brackets (MIT): a
//! closing bracket closes the nearest open bracket of its kind, implicitly
//! ending the unclosed pairs inside it; pairs that never close and closing
//! brackets with nothing to close are invalid. Every bracket pair of the
//! language pairs, but only the colorized pairs are colored and nest
//! (`closesColorized`), from `languageBracketsConfiguration.ts`.

use std::ops::Range;

use syntax::LanguageConfiguration;
use text::Rope;

/// One bracket in the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BracketToken {
    /// Byte range of the bracket.
    pub range: Range<usize>,
    /// Nesting level (0 for top-level pairs), or `None` for an unmatched
    /// bracket (Monaco's `editorBracketHighlight.unexpectedBracket`).
    pub level: Option<u32>,
    /// Nesting level among pairs with the same opening bracket (Monaco
    /// `nestingLevelOfEqualBracketType`, for
    /// `independentColorPoolPerBracketType`); 0 when unmatched.
    pub level_of_equal_bracket_type: u32,
}

/// A closed bracket pair (Monaco `BracketPairWithMinIndentationInfo`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BracketPair {
    /// Byte range of the opening bracket.
    pub open: Range<usize>,
    /// Byte range of the closing bracket.
    pub close: Range<usize>,
    /// Nesting level (0 for top-level pairs).
    pub level: u32,
    /// Nesting level among pairs with the same opening bracket.
    pub level_of_equal_bracket_type: u32,
    /// The least indentation (visible columns) of the non-blank lines
    /// after the opening bracket's, through the closing bracket's
    /// (`u32::MAX` without any).
    pub min_indentation: u32,
}

/// A bracket pair of a language as the bracket pair tree sees it (Monaco
/// `LanguageBracketsConfiguration`'s opening and closing bracket kinds).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BracketKind {
    /// The opening bracket.
    pub open: String,
    /// The closing bracket.
    pub close: String,
    /// Whether bracket pair colorization colors the pair once it closes
    /// (Monaco `closesColorized`). Pairs that never close are colored either
    /// way, as unexpected brackets.
    pub colorized: bool,
}

impl BracketKind {
    /// A colorized pair.
    pub fn colorized(open: &str, close: &str) -> Self {
        Self {
            open: open.to_string(),
            close: close.to_string(),
            colorized: true,
        }
    }
}

/// Monaco `LanguageBracketsConfiguration`: the language's `brackets`, then
/// its `colorizedBracketPairs` that aren't among them. A pair is colorized
/// when it's among `colorizedBracketPairs`, or, without them, unless it's
/// `<`…`>` (many languages also use `<` and `>` as comparison operators).
/// Pairs with an empty side are left out.
pub fn language_bracket_kinds(config: &LanguageConfiguration) -> Vec<BracketKind> {
    fn valid(pairs: &[(String, String)]) -> impl Iterator<Item = &(String, String)> {
        pairs
            .iter()
            .filter(|(open, close)| !open.is_empty() && !close.is_empty())
    }
    let colorized: Vec<&(String, String)> = match &config.colorized_bracket_pairs {
        Some(pairs) => valid(pairs).collect(),
        None => valid(config.brackets())
            .filter(|(open, close)| !(open == "<" && close == ">"))
            .collect(),
    };
    let mut kinds: Vec<BracketKind> = Vec::new();
    for (open, close) in valid(config.brackets()).chain(colorized.iter().copied()) {
        if !kinds.iter().any(|k| &k.open == open && &k.close == close) {
            kinds.push(BracketKind {
                open: open.clone(),
                close: close.clone(),
                colorized: colorized.iter().any(|(o, c)| o == open && c == close),
            });
        }
    }
    kinds
}

/// The brackets of `rope` that bracket pair colorization colors, with
/// their nesting levels. `kinds` are the language's bracket pairs (see
/// [`language_bracket_kinds`]); brackets in `skip` (sorted byte ranges of
/// strings and comments) are ignored.
pub fn colorize_brackets(
    rope: &Rope,
    kinds: &[BracketKind],
    skip: &[Range<usize>],
) -> Vec<BracketToken> {
    parse_brackets(rope, kinds, skip).0
}

/// The brackets as [`colorize_brackets`] finds them, and the closed pairs
/// of every kind, colorized or not, sorted by opening bracket, with their
/// minimum indentation (Monaco's `computeMinIndentation`).
pub fn bracket_pairs(
    rope: &Rope,
    kinds: &[BracketKind],
    skip: &[Range<usize>],
    tab_size: u32,
) -> (Vec<BracketToken>, Vec<BracketPair>) {
    let (tokens, matched) = parse_brackets(rope, kinds, skip);
    // Each row's indentation, computed for the rows inside pairs.
    let mut indents: Vec<Option<Option<u32>>> = vec![None; rope.line_count() as usize];
    let mut indent_of = |row: u32| -> Option<u32> {
        *indents[row as usize]
            .get_or_insert_with(|| crate::view::folding::indent_level(&rope.line(row), tab_size))
    };
    let mut result: Vec<BracketPair> = matched
        .into_iter()
        .map(|mut pair| {
            let first = rope.offset_to_point(pair.open.start).row + 1;
            let last = rope.offset_to_point(pair.close.start).row;
            let min_indentation = (first..=last).filter_map(&mut indent_of).min();
            pair.min_indentation = min_indentation.unwrap_or(u32::MAX);
            pair
        })
        .collect();
    result.sort_by_key(|pair| pair.open.start);
    (tokens, result)
}

/// A bracket found in the text.
struct RawBracket {
    range: Range<usize>,
    /// The index of its kind.
    kind: usize,
    is_open: bool,
    /// The bracket it pairs with.
    partner: Option<usize>,
    /// For an opening bracket, the index of the bracket that ends its pair:
    /// its partner, or the closing bracket of an enclosing pair that ends it
    /// unclosed (`usize::MAX` at the end of the text).
    end: usize,
}

/// An opening bracket whose pair encloses the brackets being looked at.
struct OpenPair {
    end: usize,
    matched: bool,
    /// Whether it counts for the colorized brackets' levels (Monaco's
    /// `colorize` in `collectBrackets`).
    colorize: bool,
    kind: usize,
}

/// An opening bracket's levels: its token's, and its pair's.
#[derive(Clone, Copy, Default)]
struct OpenLevels {
    level: u32,
    level_of_equal_bracket_type: u32,
    pair_level: u32,
    pair_level_of_equal_bracket_type: u32,
}

/// The colorized brackets, and the closed pairs of every kind (Monaco's
/// `collectBrackets` with `onlyColorizedBrackets`, and
/// `collectBracketPairs`).
fn parse_brackets(
    rope: &Rope,
    kinds: &[BracketKind],
    skip: &[Range<usize>],
) -> (Vec<BracketToken>, Vec<BracketPair>) {
    let kinds: Vec<&BracketKind> = kinds
        .iter()
        .filter(|kind| !kind.open.is_empty() && !kind.close.is_empty())
        .collect();
    if kinds.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let first_bytes: Vec<u8> = kinds
        .iter()
        .flat_map(|kind| [kind.open.as_bytes()[0], kind.close.as_bytes()[0]])
        .collect();

    // A closing bracket closes the nearest open bracket of its kind; the
    // pairs opened inside that one end unclosed.
    let mut raw: Vec<RawBracket> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut skip_ix = 0;
    rope.for_each_line(0..rope.len(), |line_start, line| {
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if !first_bytes.contains(&bytes[i]) {
                i += 1;
                continue;
            }
            let offset = line_start + i;
            while skip.get(skip_ix).is_some_and(|r| r.end <= offset) {
                skip_ix += 1;
            }
            if skip.get(skip_ix).is_some_and(|r| r.start <= offset) {
                i += 1;
                continue;
            }
            let rest = &line[i..];
            // Prefer the longest bracket starting here.
            let found = kinds
                .iter()
                .enumerate()
                .flat_map(|(ix, kind)| [(ix, true, kind.open.as_str()), (ix, false, &kind.close)])
                .filter(|(_, _, text)| rest.starts_with(text))
                .max_by_key(|(_, _, text)| text.len());
            let Some((kind, is_open, text)) = found else {
                i += 1;
                continue;
            };
            let index = raw.len();
            let mut bracket = RawBracket {
                range: offset..offset + text.len(),
                kind,
                is_open,
                partner: None,
                end: usize::MAX,
            };
            if is_open {
                stack.push(index);
            } else if let Some(depth) = stack.iter().rposition(|&open| raw[open].kind == kind) {
                for open in stack.drain(depth + 1..) {
                    raw[open].end = index;
                }
                let open = stack.pop().expect("found above");
                raw[open].partner = Some(index);
                raw[open].end = index;
                bracket.partner = Some(open);
            }
            raw.push(bracket);
            i += text.len();
        }
        true
    });

    let mut tokens: Vec<BracketToken> = Vec::new();
    let mut pairs: Vec<BracketPair> = Vec::new();
    let mut levels: Vec<OpenLevels> = vec![OpenLevels::default(); raw.len()];
    let mut open: Vec<OpenPair> = Vec::new();
    for (index, bracket) in raw.iter().enumerate() {
        while open
            .last()
            .is_some_and(|p| p.end < index || (!p.matched && p.end == index))
        {
            open.pop();
        }
        let kind = kinds[bracket.kind];
        let same_open = |p: &&OpenPair| kinds[p.kind].open == kind.open;
        if bracket.is_open {
            let matched = bracket.partner.is_some();
            // A closed pair that isn't colorized has its brackets left
            // uncolored and doesn't nest what's inside it.
            let colorize = !matched || kind.colorized;
            let at = OpenLevels {
                level: open.iter().filter(|p| p.colorize).count() as u32,
                level_of_equal_bracket_type: open
                    .iter()
                    .filter(|p| p.colorize)
                    .filter(same_open)
                    .count() as u32,
                pair_level: open.len() as u32,
                pair_level_of_equal_bracket_type: open.iter().filter(same_open).count() as u32,
            };
            if colorize {
                tokens.push(BracketToken {
                    range: bracket.range.clone(),
                    level: matched.then_some(at.level),
                    level_of_equal_bracket_type: if matched {
                        at.level_of_equal_bracket_type
                    } else {
                        0
                    },
                });
            }
            levels[index] = at;
            open.push(OpenPair {
                end: bracket.end,
                matched,
                colorize,
                kind: bracket.kind,
            });
        } else if let Some(partner) = bracket.partner {
            let at = levels[partner];
            if kind.colorized {
                tokens.push(BracketToken {
                    range: bracket.range.clone(),
                    level: Some(at.level),
                    level_of_equal_bracket_type: at.level_of_equal_bracket_type,
                });
            }
            pairs.push(BracketPair {
                open: raw[partner].range.clone(),
                close: bracket.range.clone(),
                level: at.pair_level,
                level_of_equal_bracket_type: at.pair_level_of_equal_bracket_type,
                min_indentation: u32::MAX,
            });
        } else {
            tokens.push(BracketToken {
                range: bracket.range.clone(),
                level: None,
                level_of_equal_bracket_type: 0,
            });
        }
    }
    (tokens, pairs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs() -> Vec<BracketKind> {
        [("(", ")"), ("[", "]"), ("{", "}")]
            .iter()
            .map(|(a, b)| BracketKind::colorized(a, b))
            .collect()
    }

    fn config(
        brackets: &[(&str, &str)],
        colorized: Option<&[(&str, &str)]>,
    ) -> LanguageConfiguration {
        let pairs = |p: &[(&str, &str)]| -> Vec<(String, String)> {
            p.iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect()
        };
        LanguageConfiguration {
            brackets: Some(pairs(brackets)),
            colorized_bracket_pairs: colorized.map(pairs),
            ..Default::default()
        }
    }

    /// Monaco `LanguageBracketsConfiguration`: `brackets` and
    /// `colorizedBracketPairs` together, colorized by the latter, which
    /// default to every bracket pair but `<>` (#481).
    #[test]
    fn bracket_kinds_are_the_union_of_brackets_and_colorized_pairs() {
        let summary = |config: &LanguageConfiguration| -> Vec<(String, bool)> {
            language_bracket_kinds(config)
                .into_iter()
                .map(|k| (format!("{}{}", k.open, k.close), k.colorized))
                .collect()
        };
        let s = |t: &str, c: bool| (t.to_string(), c);
        assert_eq!(
            summary(&config(&[("<", ">"), ("(", ")")], None)),
            vec![s("<>", false), s("()", true)]
        );
        assert_eq!(
            summary(&config(&[("<", ">")], Some(&[("{", "}"), ("<", ">")]))),
            vec![s("<>", true), s("{}", true)]
        );
        // The bundled HTML configuration's empty colorized pairs.
        assert_eq!(
            summary(&config(&[("{", "}"), ("", ")")], Some(&[]))),
            vec![s("{}", false)]
        );
    }

    /// A closed pair that isn't colorized pairs (for guides and
    /// `getBracketPairsInRange`) but is neither colored nor nests the
    /// colorized brackets inside it (Monaco `collectBrackets`'
    /// `closesColorized`); one that never closes is unexpected (#481).
    #[test]
    fn pairs_that_are_not_colorized() {
        let kinds = language_bracket_kinds(&config(&[("<", ">"), ("(", ")")], None));
        let colored = |text: &str| -> Vec<(String, Option<u32>)> {
            colorize_brackets(&Rope::from(text), &kinds, &[])
                .into_iter()
                .map(|t| (text[t.range].to_string(), t.level))
                .collect()
        };
        let s = |t: &str, l: Option<u32>| (t.to_string(), l);
        assert_eq!(colored("Vec<(a)>"), vec![s("(", Some(0)), s(")", Some(0))]);
        assert_eq!(
            colored("(a<(b)>)"),
            vec![
                s("(", Some(0)),
                s("(", Some(1)),
                s(")", Some(1)),
                s(")", Some(0))
            ]
        );
        // `<` never closes: `)` ends it.
        assert_eq!(
            colored("if (a < b) {}"),
            vec![s("(", Some(0)), s("<", None), s(")", Some(0))]
        );
        let text = "Vec<(a)>";
        let (_, found) = bracket_pairs(&Rope::from(text), &kinds, &[], 4);
        let summary: Vec<_> = found
            .iter()
            .map(|p| (&text[p.open.clone()], &text[p.close.clone()], p.level))
            .collect();
        assert_eq!(summary, vec![("<", ">", 0), ("(", ")", 1)]);
    }

    fn levels(text: &str, skip: &[Range<usize>]) -> Vec<(char, Option<u32>)> {
        colorize_brackets(&Rope::from(text), &pairs(), skip)
            .into_iter()
            .map(|t| (text[t.range.clone()].chars().next().unwrap(), t.level))
            .collect()
    }

    #[test]
    fn nesting_levels() {
        assert_eq!(
            levels("f(a[b{c}], {})", &[]),
            vec![
                ('(', Some(0)),
                ('[', Some(1)),
                ('{', Some(2)),
                ('}', Some(2)),
                (']', Some(1)),
                ('{', Some(1)),
                ('}', Some(1)),
                (')', Some(0)),
            ]
        );
        // Across lines.
        assert_eq!(
            levels("{\n  (\n  )\n}", &[]),
            vec![
                ('{', Some(0)),
                ('(', Some(1)),
                (')', Some(1)),
                ('}', Some(0))
            ]
        );
    }

    #[test]
    fn unmatched_brackets() {
        // A stray closing bracket.
        assert_eq!(levels("a)", &[]), vec![(')', None)]);
        // `[` never closes: `)` closes `(` and ends it.
        assert_eq!(
            levels("([)", &[]),
            vec![('(', Some(0)), ('[', None), (')', Some(0))]
        );
        // Unclosed at the end.
        assert_eq!(
            levels("{(}", &[]),
            vec![('{', Some(0)), ('(', None), ('}', Some(0))]
        );
        assert_eq!(levels("(", &[]), vec![('(', None)]);
    }

    #[test]
    fn pairs_with_min_indentation() {
        let text = "fn f() {\n    if x {\n        y\n\n    }\n}";
        let (_, found) = bracket_pairs(&Rope::from(text), &pairs(), &[], 4);
        let summary: Vec<_> = found
            .iter()
            .map(|p| (&text[p.open.clone()], p.level, p.min_indentation))
            .collect();
        assert_eq!(summary, vec![("(", 0, u32::MAX), ("{", 0, 0), ("{", 1, 4)]);
    }

    #[test]
    fn levels_of_equal_bracket_type() {
        let text = "f(a[b(c)], (d))";
        let found: Vec<_> = colorize_brackets(&Rope::from(text), &pairs(), &[])
            .into_iter()
            .map(|t| {
                (
                    text[t.range.clone()].chars().next().unwrap(),
                    t.level_of_equal_bracket_type,
                )
            })
            .collect();
        assert_eq!(
            found,
            vec![
                ('(', 0),
                ('[', 0),
                ('(', 1),
                (')', 1),
                (']', 0),
                ('(', 1),
                (')', 1),
                (')', 0),
            ]
        );
    }

    #[test]
    fn skips_strings_and_comments() {
        let text = "f(\"(\") // )";
        assert_eq!(
            levels(text, &[2..5, 7..11]),
            vec![('(', Some(0)), (')', Some(0))]
        );
    }
}
