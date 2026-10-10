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

/// The brackets of `rope` with their nesting levels. `pairs` are the
/// language's (colorized) bracket pairs; brackets in `skip` (sorted byte
/// ranges of strings and comments) are ignored.
pub fn colorize_brackets(
    rope: &Rope,
    pairs: &[(String, String)],
    skip: &[Range<usize>],
) -> Vec<BracketToken> {
    let pairs: Vec<(&str, &str)> = pairs
        .iter()
        .filter(|(open, close)| !open.is_empty() && !close.is_empty())
        .map(|(open, close)| (open.as_str(), close.as_str()))
        .collect();
    if pairs.is_empty() {
        return Vec::new();
    }
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
                stack.pop();
                tokens.push(BracketToken {
                    range,
                    level: Some(depth as u32),
                });
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
    tokens
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
    fn skips_strings_and_comments() {
        let text = "f(\"(\") // )";
        assert_eq!(
            levels(text, &[2..5, 7..11]),
            vec![('(', Some(0)), (')', Some(0))]
        );
    }
}
