//! Word segmentation for Monaco's `wordSegmenterLocales`.
//!
//! Ports the `Intl.Segmenter` half of Monaco's
//! `src/vs/editor/common/core/wordCharacterClassifier.ts` (MIT): lines are
//! split into word-like segments (`IntlWordSegmentData`) with ICU4X's word
//! segmenter, which finds words in languages written without spaces, such
//! as Japanese, Chinese and Thai. Without the `word-segmenter` cargo feature
//! there is no segmenter and the locales have no effect.

use std::cell::RefCell;

/// A word-like segment of a line (Monaco `IntlWordSegmentData`), in UTF-16
/// code units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct IntlWord {
    /// Where the segment starts (Monaco `index`).
    pub index: u32,
    /// Where the segment ends (Monaco `index + segment.length`).
    pub end: u32,
}

/// Monaco's `Intl.Segmenter` with `granularity: 'word'`, plus the
/// `WordCharacterClassifier` cache of the last line's word segments.
pub(crate) struct IntlWordSegmenter {
    #[cfg(feature = "word-segmenter")]
    segmenter: icu_segmenter::WordSegmenter,
    #[cfg(not(feature = "word-segmenter"))]
    never: std::convert::Infallible,
    cache: RefCell<Option<(Vec<u16>, Vec<IntlWord>)>>,
}

impl IntlWordSegmenter {
    /// A segmenter for the first valid BCP 47 tag in `locales`, or `None` if
    /// there is none (Monaco `WordSegmenterLocales.validate` drops the tags
    /// `Intl.Segmenter` doesn't support) or the `word-segmenter` feature is
    /// off.
    #[cfg(feature = "word-segmenter")]
    pub(crate) fn new<S: AsRef<str>>(locales: &[S]) -> Option<Self> {
        use icu_segmenter::{WordSegmenter, options::WordBreakOptions};
        let locale = locales
            .iter()
            .find_map(|l| icu_locale_core::Locale::try_from_str(l.as_ref().trim()).ok())?;
        let mut options = WordBreakOptions::default();
        options.content_locale = Some(&locale.id);
        let segmenter = WordSegmenter::try_new_auto(options).ok()?;
        Some(Self {
            segmenter,
            cache: RefCell::new(None),
        })
    }

    /// Always `None`: the `word-segmenter` feature is off.
    #[cfg(not(feature = "word-segmenter"))]
    pub(crate) fn new<S: AsRef<str>>(_locales: &[S]) -> Option<Self> {
        None
    }

    /// Monaco `_getIntlSegmenterWordsOnLine`: the word-like segments of
    /// `line`, computed again only when the line changes.
    fn with_words<R>(&self, line: &[u16], f: impl FnOnce(&[IntlWord]) -> R) -> R {
        let mut cache = self.cache.borrow_mut();
        if !cache.as_ref().is_some_and(|(cached, _)| cached == line) {
            *cache = Some((line.to_vec(), self.segment(line)));
        }
        let (_, words) = cache.as_ref().expect("filled above");
        f(words)
    }

    /// Monaco `_filterWordSegments(segmenter.segment(line))`.
    #[cfg(feature = "word-segmenter")]
    fn segment(&self, line: &[u16]) -> Vec<IntlWord> {
        let mut words = Vec::new();
        let mut start = 0;
        let breaks = self.segmenter.as_borrowed().segment_utf16(line);
        for (end, word_type) in breaks.iter_with_word_type() {
            if end > start && word_type.is_word_like() {
                words.push(IntlWord {
                    index: start as u32,
                    end: end as u32,
                });
            }
            start = end;
        }
        words
    }

    #[cfg(not(feature = "word-segmenter"))]
    fn segment(&self, _line: &[u16]) -> Vec<IntlWord> {
        match self.never {}
    }

    /// Monaco `findPrevIntlWordBeforeOrAtOffset`: the last word starting at
    /// or before `offset`.
    pub(crate) fn find_prev_word_before_or_at_offset(
        &self,
        line: &[u16],
        offset: i64,
    ) -> Option<IntlWord> {
        self.with_words(line, |words| {
            words
                .iter()
                .take_while(|w| w.index as i64 <= offset)
                .last()
                .copied()
        })
    }

    /// Monaco `findNextIntlWordAtOrAfterOffset`: the first word starting at
    /// or after `offset`.
    pub(crate) fn find_next_word_at_or_after_offset(
        &self,
        line: &[u16],
        offset: i64,
    ) -> Option<IntlWord> {
        self.with_words(line, |words| {
            words.iter().find(|w| w.index as i64 >= offset).copied()
        })
    }
}

#[cfg(all(test, feature = "word-segmenter"))]
mod tests {
    use super::*;

    fn words(locale: &str, line: &str) -> Vec<String> {
        let segmenter = IntlWordSegmenter::new(&[locale]).expect("valid locale");
        let units: Vec<u16> = line.encode_utf16().collect();
        segmenter.with_words(&units, |words| {
            words
                .iter()
                .map(|w| String::from_utf16_lossy(&units[w.index as usize..w.end as usize]))
                .collect()
        })
    }

    #[test]
    fn japanese_words() {
        assert_eq!(
            words("ja", "/* これはテストです /*"),
            ["これ", "は", "テスト", "です"]
        );
    }

    #[test]
    fn chinese_words() {
        assert_eq!(words("zh-CN", "我们是学生"), ["我们", "是", "学生"]);
    }

    #[test]
    fn thai_words() {
        assert_eq!(words("th", "ทุกสองสัปดาห์"), ["ทุก", "สอง", "สัปดาห์"]);
    }

    #[test]
    fn latin_words_and_numbers_are_word_like() {
        assert_eq!(words("ja", "let a1 = 42;"), ["let", "a1", "42"]);
    }

    #[test]
    fn invalid_locales_are_ignored() {
        assert!(IntlWordSegmenter::new::<&str>(&[]).is_none());
        assert!(IntlWordSegmenter::new(&["", "not a locale!"]).is_none());
        assert!(IntlWordSegmenter::new(&["", "ja"]).is_some());
        assert!(IntlWordSegmenter::new(&["zh-Hant-TW"]).is_some());
    }

    #[test]
    fn find_prev_and_next() {
        let segmenter = IntlWordSegmenter::new(&["ja"]).expect("valid locale");
        let line: Vec<u16> = "これはテスト".encode_utf16().collect();
        let prev = |o| segmenter.find_prev_word_before_or_at_offset(&line, o);
        let next = |o| segmenter.find_next_word_at_or_after_offset(&line, o);
        assert_eq!(prev(-1), None);
        assert_eq!(prev(0), Some(IntlWord { index: 0, end: 2 }));
        assert_eq!(prev(1), Some(IntlWord { index: 0, end: 2 }));
        assert_eq!(prev(2), Some(IntlWord { index: 2, end: 3 }));
        assert_eq!(prev(10), Some(IntlWord { index: 3, end: 6 }));
        assert_eq!(next(0), Some(IntlWord { index: 0, end: 2 }));
        assert_eq!(next(1), Some(IntlWord { index: 2, end: 3 }));
        assert_eq!(next(4), None);
    }
}
