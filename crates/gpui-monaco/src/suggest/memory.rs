//! Which suggestion to select first (Monaco `editor.suggestSelection`).
//!
//! Ports `Memory`, `NoMemory`, `LRUMemory` and `PrefixMemory` from
//! `src/vs/editor/contrib/suggest/browser/suggestMemory.ts` (MIT), kept in
//! memory (Monaco also persists them).

use std::collections::{HashMap, VecDeque};

use crate::providers::CompletionItemKind;

/// Monaco `editor.suggestSelection`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SuggestSelection {
    /// The first suggestion (or the first preselected one).
    #[default]
    First,
    /// The one used most recently, unless typing after whitespace.
    RecentlyUsed,
    /// The one last used after typing the same prefix.
    RecentlyUsedByPrefix,
}

/// A filtered suggestion, as the memories see it.
#[derive(Clone, Debug)]
pub struct Candidate<'a> {
    /// Its label (Monaco `textLabel`).
    pub label: &'a str,
    /// Its kind.
    pub kind: CompletionItemKind,
    /// The text it inserts.
    pub insert_text: &'a str,
    /// `preselect`.
    pub preselect: bool,
    /// Its filter score (Monaco `score[0]`).
    pub score: i32,
}

/// Where the suggestions are shown.
#[derive(Clone, Copy, Debug)]
pub struct MemoryContext<'a> {
    /// The model's language id.
    pub language: &'a str,
    /// The cursor's line.
    pub line: &'a str,
    /// The cursor's 0-based UTF-16 column.
    pub column: u32,
    /// The word before the cursor (Monaco `getWordUntilPosition`).
    pub word_until: &'a str,
}

/// Monaco `Memory.select`: the first preselected item among those with
/// the top score, else 0.
pub fn base_select(items: &[Candidate]) -> usize {
    let Some(top) = items.first().map(|i| i.score) else {
        return 0;
    };
    items
        .iter()
        .take_while(|item| item.score == top)
        .position(|item| item.preselect)
        .unwrap_or(0)
}

#[derive(Clone, Debug)]
struct MemItem {
    kind: CompletionItemKind,
    insert_text: String,
    touch: u64,
}

/// Monaco `LRUMemory`: per language and label, the last used kind and
/// insert text; at most 300 entries, trimmed to 66% when full.
#[derive(Default)]
pub struct LruMemory {
    entries: HashMap<String, MemItem>,
    /// Keys, least recently set first.
    order: VecDeque<String>,
    seq: u64,
}

const LRU_LIMIT: usize = 300;
const LRU_RATIO: f64 = 0.66;

impl LruMemory {
    /// Remembers that `label` was accepted.
    pub fn memorize(
        &mut self,
        cx: &MemoryContext,
        label: &str,
        kind: CompletionItemKind,
        insert_text: &str,
    ) {
        let key = format!("{}/{}", cx.language, label);
        self.order.retain(|k| *k != key);
        self.order.push_back(key.clone());
        self.entries.insert(
            key,
            MemItem {
                kind,
                insert_text: insert_text.to_string(),
                touch: self.seq,
            },
        );
        self.seq += 1;
        if self.entries.len() > LRU_LIMIT {
            let keep = (LRU_LIMIT as f64 * LRU_RATIO).round() as usize;
            while self.entries.len() > keep {
                if let Some(old) = self.order.pop_front() {
                    self.entries.remove(&old);
                }
            }
        }
    }

    /// Monaco `LRUMemory.select`.
    pub fn select(&self, cx: &MemoryContext, items: &[Candidate]) -> usize {
        if items.is_empty() {
            return 0;
        }
        // Monaco: `line.substr(column - 10, column - 1)` with 1-based
        // columns, JavaScript `substr` semantics.
        let units: Vec<u16> = cx.line.encode_utf16().collect();
        let column = cx.column as i64 + 1;
        let start = column - 10;
        let start = if start < 0 {
            (units.len() as i64 + start).max(0)
        } else {
            start.min(units.len() as i64)
        } as usize;
        let len = (column - 1).max(0) as usize;
        let end = (start + len).min(units.len());
        let suffix = String::from_utf16_lossy(&units[start..end]);
        if suffix.chars().last().is_some_and(char::is_whitespace) {
            return base_select(items);
        }
        let top = items[0].score;
        let mut recency = None;
        let mut seq: i64 = -1;
        for (i, item) in items.iter().enumerate() {
            if item.score != top {
                break;
            }
            let key = format!("{}/{}", cx.language, item.label);
            if let Some(mem) = self.entries.get(&key)
                && mem.touch as i64 > seq
                && mem.kind == item.kind
                && mem.insert_text == item.insert_text
            {
                seq = mem.touch as i64;
                recency = Some(i);
            }
            if item.preselect {
                return i;
            }
        }
        recency.unwrap_or(0)
    }
}

/// Monaco `PrefixMemory`: per language and typed prefix, the last used
/// kind and insert text; a longer prefix finds the longest stored one.
#[derive(Default)]
pub struct PrefixMemory {
    entries: HashMap<String, MemItem>,
    seq: u64,
}

impl PrefixMemory {
    /// Remembers that the item was accepted after typing `cx.word_until`.
    pub fn memorize(&mut self, cx: &MemoryContext, kind: CompletionItemKind, insert_text: &str) {
        let key = format!("{}/{}", cx.language, cx.word_until);
        self.entries.insert(
            key,
            MemItem {
                kind,
                insert_text: insert_text.to_string(),
                touch: self.seq,
            },
        );
        self.seq += 1;
    }

    /// Monaco `PrefixMemory.select`.
    pub fn select(&self, cx: &MemoryContext, items: &[Candidate]) -> usize {
        if cx.word_until.is_empty() {
            return base_select(items);
        }
        let key = format!("{}/{}", cx.language, cx.word_until);
        // The exact key, else the longest stored key that's a prefix of it
        // (Monaco `TernarySearchTree.findSubstr`).
        let found = self.entries.get(&key).or_else(|| {
            key.char_indices()
                .map(|(i, _)| &key[..i])
                .rev()
                .find_map(|prefix| self.entries.get(prefix))
        });
        if let Some(mem) = found
            && let Some(ix) = items
                .iter()
                .position(|item| item.kind == mem.kind && item.insert_text == mem.insert_text)
        {
            return ix;
        }
        base_select(items)
    }
}

/// The memories for each [`SuggestSelection`] mode, shared by all editors.
#[derive(Default)]
pub struct SuggestMemories {
    /// `recentlyUsed`.
    pub lru: LruMemory,
    /// `recentlyUsedByPrefix`.
    pub prefix: PrefixMemory,
}

impl SuggestMemories {
    /// The index to select in `items` (Monaco `SuggestMemoryService.select`).
    pub fn select(&self, mode: SuggestSelection, cx: &MemoryContext, items: &[Candidate]) -> usize {
        match mode {
            SuggestSelection::First => base_select(items),
            SuggestSelection::RecentlyUsed => self.lru.select(cx, items),
            SuggestSelection::RecentlyUsedByPrefix => self.prefix.select(cx, items),
        }
    }

    /// Remembers an accepted item (Monaco `SuggestMemoryService.memorize`).
    pub fn memorize(&mut self, mode: SuggestSelection, cx: &MemoryContext, item: &Candidate) {
        match mode {
            SuggestSelection::First => {}
            SuggestSelection::RecentlyUsed => {
                self.lru
                    .memorize(cx, item.label, item.kind, item.insert_text)
            }
            SuggestSelection::RecentlyUsedByPrefix => {
                self.prefix.memorize(cx, item.kind, item.insert_text)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! Ported from Monaco's
    //! `contrib/suggest/test/browser/suggestMemory.test.ts`.
    use super::*;

    fn item(label: &str) -> Candidate<'_> {
        Candidate {
            label,
            kind: CompletionItemKind::Property,
            insert_text: label,
            preselect: false,
            score: 0,
        }
    }

    /// Monaco's 1-based `(lineNumber, column)` in `lines` (ASCII text).
    fn at<'a>(lines: &[&'a str], line: usize, column: u32) -> MemoryContext<'a> {
        let text = lines[line - 1];
        let column = column - 1;
        let before = &text[..column as usize];
        let start = before
            .rfind(|c: char| !(c.is_alphanumeric() || c == '_'))
            .map_or(0, |i| i + 1);
        let word: &'a str = &text[start..column as usize];
        MemoryContext {
            language: "plaintext",
            line: text,
            column,
            word_until: word,
        }
    }

    const BUFFER: [&str; 3] = ["This is some text.", "this.", "foo: ,"];

    #[test]
    fn abstract_memory_select_and_selection_boost() {
        let mut items = vec![item("fazz"), item("bazz"), item("bazz"), item("bazz")];
        items[1].preselect = true;
        items[2].preselect = true;
        let cx = at(&BUFFER, 1, 1);
        assert_eq!(base_select(&items), 1);
        assert_eq!(LruMemory::default().select(&cx, &items), 1);
        assert_eq!(PrefixMemory::default().select(&cx, &items), 1);
    }

    #[test]
    fn no_memory() {
        assert_eq!(base_select(&[item("foo"), item("bar")]), 0);
        assert_eq!(base_select(&[]), 0);
    }

    #[test]
    fn lru_memory() {
        let items = [item("foo"), item("bar")];
        let pos = at(&BUFFER, 2, 6);
        let mut mem = LruMemory::default();
        mem.memorize(&pos, "bar", items[1].kind, "bar");
        assert_eq!(mem.select(&pos, &items), 1);
        assert_eq!(mem.select(&at(&BUFFER, 1, 3), &items), 0);
        mem.memorize(&pos, "foo", items[0].kind, "foo");
        assert_eq!(mem.select(&pos, &items), 0);
        assert_eq!(mem.select(&pos, &[item("new"), item("bar")]), 1);
        assert_eq!(mem.select(&pos, &[item("new1"), item("new2")]), 0);
    }

    #[test]
    fn recently_used_is_sticky_78571() {
        let mut items = [item("gamma"), item("game")];
        let mut mem = LruMemory::default();
        let lines = ["    foo."];
        mem.memorize(&at(&lines, 1, 1), "game", items[1].kind, "game");
        // Leading whitespace: recent items are ignored.
        assert_eq!(mem.select(&at(&lines, 1, 2), &items), 0);
        mem.memorize(&at(&lines, 1, 9), "game", items[1].kind, "game");
        assert_eq!(mem.select(&at(&lines, 1, 9), &items), 1);
        let lines = ["    foo.g"];
        assert_eq!(mem.select(&at(&lines, 1, 10), &items), 1);
        items[0].score = 10;
        assert_eq!(mem.select(&at(&lines, 1, 10), &items), 0);
    }

    #[test]
    fn whitespace_prefixes_43429() {
        let items = [item("foo"), item("bar")];
        let mut mem = LruMemory::default();
        mem.memorize(&at(&BUFFER, 2, 6), "bar", items[1].kind, "bar");
        assert_eq!(mem.select(&at(&BUFFER, 2, 6), &items), 1);
        assert_eq!(mem.select(&at(&BUFFER, 3, 5), &items), 0); // foo: |,
        assert_eq!(mem.select(&at(&BUFFER, 3, 6), &items), 1); // foo: ,|
    }

    #[test]
    fn prefix_memory() {
        let lines = ["constructor"];
        let items = [
            item("console"),
            item("const"),
            item("constructor"),
            item("constant"),
        ];
        let mut mem = PrefixMemory::default();
        mem.memorize(&at(&lines, 1, 2), items[1].kind, "const"); // c -> const
        mem.memorize(&at(&lines, 1, 3), items[0].kind, "console"); // co -> console
        mem.memorize(&at(&lines, 1, 4), items[2].kind, "constructor"); // con -> constructor
        assert_eq!(mem.select(&at(&lines, 1, 1), &items), 0);
        assert_eq!(mem.select(&at(&lines, 1, 2), &items), 1);
        assert_eq!(mem.select(&at(&lines, 1, 3), &items), 0);
        assert_eq!(mem.select(&at(&lines, 1, 4), &items), 2);
        assert_eq!(mem.select(&at(&lines, 1, 7), &items), 2); // find substr
    }
}
