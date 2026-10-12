//! Monaco's `WordDistance`
//! (`src/vs/editor/contrib/suggest/browser/wordDistance.ts`): with
//! `suggest.localityBonus`, how many bracket blocks out from the cursor a
//! suggestion's word is used. The completion model sorts by it after the
//! score, so among equal matches (word-based suggestions included) words
//! used near the cursor come first.

use std::collections::HashMap;
use std::ops::Range;

use syntax::fancy_regex::Regex;
use text::{PointUtf16, Rope};

use crate::providers::{CompletionItem, CompletionItemKind};

/// Monaco's distance for keywords and words not used in the innermost
/// block (`2 << 20`).
pub const FAR: u32 = 2 << 20;

/// The words around a position and the bracket blocks they're in.
#[derive(Clone, Debug)]
pub struct WordDistance {
    /// The bracket blocks around the position, innermost first (Monaco's
    /// `BracketSelectionRangeProvider`).
    blocks: Vec<Range<PointUtf16>>,
    /// Where each word of the innermost block's lines is, in order
    /// (Monaco `computeWordRanges`).
    words: HashMap<String, Vec<Range<PointUtf16>>>,
}

impl WordDistance {
    /// Monaco `WordDistance.create`: the words of the lines of the
    /// innermost of `blocks` (the bracket selection ranges at the cursor,
    /// innermost first; its last line left out, as in Monaco), found with
    /// `word_pattern`, except numbers and `current` (the word typed so far,
    /// Monaco `getWordUntilPosition`). `None` without a block (Monaco's
    /// `WordDistance.None`: every distance is 0).
    pub fn new(
        rope: &Rope,
        blocks: Vec<Range<PointUtf16>>,
        word_pattern: &Regex,
        current: &str,
    ) -> Option<Self> {
        let block = blocks.first()?;
        let mut words: HashMap<String, Vec<Range<PointUtf16>>> = HashMap::new();
        for row in block.start.row..block.end.row {
            let line = rope.line(row);
            let mut start = 0;
            while start < line.len() {
                let Ok(Some(found)) = word_pattern.find_from_pos(&line, start) else {
                    break;
                };
                start = if found.end() > found.start() {
                    found.end()
                } else {
                    found.end() + line[found.end()..].chars().next().map_or(1, char::len_utf8)
                };
                let word = found.as_str();
                if word.is_empty() || word.parse::<f64>().is_ok() {
                    continue;
                }
                let column = |byte: usize| line[..byte].encode_utf16().count() as u32;
                words.entry(word.to_string()).or_default().push(
                    PointUtf16::new(row, column(found.start()))
                        ..PointUtf16::new(row, column(found.end())),
                );
            }
        }
        words.remove(current);
        Some(Self { blocks, words })
    }

    /// Monaco `WordDistance.distance`: for `item` requested at `anchor`, how
    /// many blocks its word's use nearest before `anchor` is outside of (0
    /// in the innermost); [`FAR`] for keywords and words not used there.
    pub fn distance(&self, anchor: PointUtf16, item: &CompletionItem) -> u32 {
        if item.kind == CompletionItemKind::Keyword {
            return FAR;
        }
        let Some(uses) = self.words.get(item.label.as_ref()) else {
            return FAR;
        };
        // The use at `anchor`, else the last one before it (Monaco's
        // `binarySearch` with `compareRangesUsingStarts`).
        let best = match uses.binary_search_by(|r| r.start.cmp(&anchor).then(r.end.cmp(&anchor))) {
            Ok(ix) => &uses[ix],
            Err(ix) => &uses[ix.saturating_sub(1)],
        };
        let mut distance = self.blocks.len() as u32;
        for block in &self.blocks {
            if !(block.start <= best.start && best.end <= block.end) {
                break;
            }
            distance -= 1;
        }
        distance
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Monaco's `wordDistance.test.ts` setup: `{}`, `[]` and `()` brackets,
    /// the cursor after the `a` on line 2.
    fn distance() -> WordDistance {
        let rope = Rope::from("function abc(aa, ab){\na\n}");
        let position = PointUtf16::new(1, 1);
        let blocks = crate::cursor::smart_select::bracket_selection_ranges(
            &rope,
            &syntax::LanguageConfiguration::default(),
            position,
            &|_| false,
        );
        WordDistance::new(
            &rope,
            blocks,
            crate::cursor::in_place_replace::default_word_regex(),
            "a",
        )
        .unwrap()
    }

    fn item(label: &str) -> CompletionItem {
        CompletionItem::new(label, CompletionItemKind::Variable)
    }

    /// Monaco: "Suggest locality bonus can boost current word #90515".
    #[test]
    fn locality_bonus_can_boost_current_word() {
        let distance = distance();
        let position = PointUtf16::new(1, 1);
        let d1 = distance.distance(position, &item("a"));
        let d2 = distance.distance(position, &item("aa"));
        let d3 = distance.distance(position, &item("ab"));
        assert!(d1 > d2);
        assert_eq!(d2, d3);
    }

    #[test]
    fn keywords_and_unused_words_are_far() {
        let distance = distance();
        let position = PointUtf16::new(1, 1);
        assert_eq!(distance.distance(position, &item("zzz")), FAR);
        let keyword = CompletionItem::new("abc", CompletionItemKind::Keyword);
        assert_eq!(distance.distance(position, &keyword), FAR);
        // `abc` is used on the block's first line, outside its braces.
        assert!(distance.distance(position, &item("abc")) < FAR);
    }
}
