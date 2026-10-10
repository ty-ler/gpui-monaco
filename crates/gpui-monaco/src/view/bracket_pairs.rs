//! Bracket pair nesting for colorization.
//!
//! Follows how Monaco's bracket pair parser
//! (`src/vs/editor/common/model/bracketPairsTextModelPart/bracketPairsTree/parser.ts`)
//! and `collectBrackets` (`bracketPairsTree.ts`) classify brackets (MIT): a
//! closing bracket closes the nearest open bracket of its kind, implicitly
//! ending the unclosed pairs inside it; pairs that never close and closing
//! brackets with nothing to close are invalid.

use std::ops::Range;

use text::Rope;

/// One bracket in the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BracketToken {
    /// Byte range of the bracket.
    pub range: Range<usize>,
    /// Nesting level (0 for top-level pairs), or `None` for an unmatched
    /// bracket (Monaco's `editorBracketHighlight.unexpectedBracket`).
    pub level: Option<u32>,
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
    /// The least indentation (visible columns) of the non-blank lines
    /// after the opening bracket's, through the closing bracket's
    /// (`u32::MAX` without any).
    pub min_indentation: u32,
}

/// The brackets of `rope` with their nesting levels. `pairs` are the
/// language's (colorized) bracket pairs; brackets in `skip` (sorted byte
/// ranges of strings and comments) are ignored.
pub fn colorize_brackets(
    rope: &Rope,
    pairs: &[(String, String)],
    skip: &[Range<usize>],
) -> Vec<BracketToken> {
    parse_brackets(rope, pairs, skip).0
}

/// The brackets as [`colorize_brackets`] finds them, and the closed pairs,
/// sorted by opening bracket, with their minimum indentation (Monaco's
/// `computeMinIndentation`).
pub fn bracket_pairs(
    rope: &Rope,
    pairs: &[(String, String)],
    skip: &[Range<usize>],
    tab_size: u32,
) -> (Vec<BracketToken>, Vec<BracketPair>) {
    let (tokens, matched) = parse_brackets(rope, pairs, skip);
    // Each row's indentation, computed for the rows inside pairs.
    let mut indents: Vec<Option<Option<u32>>> = vec![None; rope.line_count() as usize];
    let mut indent_of = |row: u32| -> Option<u32> {
        *indents[row as usize]
            .get_or_insert_with(|| crate::view::folding::indent_level(&rope.line(row), tab_size))
    };
    let mut result: Vec<BracketPair> = matched
        .into_iter()
        .filter_map(|(open, close)| {
            let level = tokens[open].level?;
            let open = tokens[open].range.clone();
            let close = tokens[close].range.clone();
            let first = rope.offset_to_point(open.start).row + 1;
            let last = rope.offset_to_point(close.start).row;
            let min_indentation = (first..=last).filter_map(&mut indent_of).min();
            Some(BracketPair {
                open,
                close,
                level,
                min_indentation: min_indentation.unwrap_or(u32::MAX),
            })
        })
        .collect();
    result.sort_by_key(|pair| pair.open.start);
    (tokens, result)
}

/// The brackets, and the (open, close) token indices of the closed pairs.
fn parse_brackets(
    rope: &Rope,
    pairs: &[(String, String)],
    skip: &[Range<usize>],
) -> (Vec<BracketToken>, Vec<(usize, usize)>) {
    let pairs: Vec<(&str, &str)> = pairs
        .iter()
        .filter(|(open, close)| !open.is_empty() && !close.is_empty())
        .map(|(open, close)| (open.as_str(), close.as_str()))
        .collect();
    if pairs.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let mut matched = Vec::new();
    let first_bytes: Vec<u8> = pairs
        .iter()
        .flat_map(|(open, close)| [open.as_bytes()[0], close.as_bytes()[0]])
        .collect();

    let mut tokens: Vec<BracketToken> = Vec::new();
    // (pair index, token index) of open brackets.
    let mut stack: Vec<(usize, usize)> = Vec::new();
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
            let found = pairs
                .iter()
                .enumerate()
                .flat_map(|(ix, (open, close))| [(ix, true, *open), (ix, false, *close)])
                .filter(|(_, _, text)| rest.starts_with(text))
                .max_by_key(|(_, _, text)| text.len());
            let Some((pair, is_open, text)) = found else {
                i += 1;
                continue;
            };
            let range = offset..offset + text.len();
            if is_open {
                tokens.push(BracketToken {
                    range,
                    level: Some(stack.len() as u32),
                });
                stack.push((pair, tokens.len() - 1));
            } else if let Some(depth) = stack.iter().rposition(|(p, _)| *p == pair) {
                // Pairs opened inside this one never closed.
                for (_, token) in stack.drain(depth + 1..) {
                    tokens[token].level = None;
                }
                let (_, open) = stack.pop().expect("found above");
                tokens.push(BracketToken {
                    range,
                    level: Some(depth as u32),
                });
                matched.push((open, tokens.len() - 1));
            } else {
                tokens.push(BracketToken { range, level: None });
            }
            i += text.len();
        }
        true
    });
    for (_, token) in stack {
        tokens[token].level = None;
    }
    (tokens, matched)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs() -> Vec<(String, String)> {
        [("(", ")"), ("[", "]"), ("{", "}")]
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
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
    fn skips_strings_and_comments() {
        let text = "f(\"(\") // )";
        assert_eq!(
            levels(text, &[2..5, 7..11]),
            vec![('(', Some(0)), (')', Some(0))]
        );
    }
}
