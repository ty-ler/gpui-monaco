//! Unicode highlighting (Monaco `UnicodeTextModelHighlighter`): finds
//! invisible, ambiguous (confusable) and non-basic-ASCII characters in a
//! text, plus the confusable and invisible character tables it relies on.
//!
//! Ports `unicodeTextModelHighlighter.ts` and `strings.ts`
//! `AmbiguousCharacters`/`InvisibleCharacters` (MIT).

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::ops::Range;
use std::sync::{Arc, LazyLock, Mutex};

use serde::Deserialize;
use serde::de::{Deserializer, MapAccess, Visitor};
use text::{PointUtf16, Rope};

/// Monaco's `MAX_RESULT_LENGTH`: at most this many ranges are returned.
pub const MAX_RESULT_COUNT: usize = 1000;

/// Monaco's code point tables, stored as `{ locale: [..] }` objects.
/// Key order matters (`getLocales`), so this keeps it.
struct LocaleTable(Vec<(String, Vec<u32>)>);

impl<'de> Deserialize<'de> for LocaleTable {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct TableVisitor;

        impl<'de> Visitor<'de> for TableVisitor {
            type Value = LocaleTable;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("an object of code point arrays")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<LocaleTable, A::Error> {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry::<String, Vec<u32>>()? {
                    entries.push(entry);
                }
                Ok(LocaleTable(entries))
            }
        }

        deserializer.deserialize_map(TableVisitor)
    }
}

fn parse_table(json: &str) -> Vec<(String, Vec<u32>)> {
    serde_json::from_str::<LocaleTable>(json)
        .expect("embedded unicode data is valid")
        .0
}

/// `ambiguousCharacterData`: per locale, `[code point, ascii code point, ..]`
/// pairs (generated with `hediet/vscode-unicode-data`).
static AMBIGUOUS_DATA: LazyLock<Vec<(String, Vec<u32>)>> =
    LazyLock::new(|| parse_table(include_str!("unicode_highlight/ambiguous.json")));

static LOCALES: LazyLock<Vec<String>> = LazyLock::new(|| {
    AMBIGUOUS_DATA
        .iter()
        .map(|(locale, _)| locale)
        .filter(|locale| !locale.starts_with('_'))
        .cloned()
        .collect()
});

/// Instances by joined locales. Monaco keeps only the last one
/// (`LRUCachedFunction`); this keeps a few so alternating callers (such as
/// [`compute_unicode_highlight_reason`]) don't rebuild maps.
static AMBIGUOUS_CACHE: LazyLock<Mutex<HashMap<String, Arc<AmbiguousCharacters>>>> =
    LazyLock::new(Default::default);
const AMBIGUOUS_CACHE_LIMIT: usize = 64;

/// The union of every locale's invisible characters.
static INVISIBLE_DATA: LazyLock<HashSet<u32>> = LazyLock::new(|| {
    parse_table(include_str!("unicode_highlight/invisible.json"))
        .into_iter()
        .flat_map(|(_, code_points)| code_points)
        .collect()
});

/// Characters that look like basic ASCII ones (Monaco
/// `AmbiguousCharacters`), for a set of locales.
#[derive(Debug)]
pub struct AmbiguousCharacters {
    confusable_dictionary: HashMap<u32, u32>,
}

impl AmbiguousCharacters {
    /// Monaco `getInstance`: the confusables shared by all known `locales`
    /// (`_default` when none is known), merged over the common ones.
    pub fn get_instance<I, S>(locales: I) -> Arc<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let key = locales
            .into_iter()
            .map(|locale| locale.as_ref().to_string())
            .collect::<Vec<_>>()
            .join(",");
        let mut cache = AMBIGUOUS_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(instance) = cache.get(&key) {
            return instance.clone();
        }
        let instance = Arc::new(Self::compute(&key));
        if cache.len() >= AMBIGUOUS_CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(key, instance.clone());
        instance
    }

    fn compute(locales: &str) -> Self {
        fn array_to_map(array: &[u32]) -> HashMap<u32, u32> {
            let (pairs, _) = array.as_chunks::<2>();
            pairs.iter().map(|&[key, value]| (key, value)).collect()
        }

        let data = &*AMBIGUOUS_DATA;
        let lookup = |locale: &str| {
            data.iter()
                .find(|(key, _)| key == locale)
                .map(|(_, array)| array.as_slice())
        };

        let mut filtered_locales: Vec<&str> = locales
            .split(',')
            .filter(|locale| !locale.starts_with('_') && lookup(locale).is_some())
            .collect();
        if filtered_locales.is_empty() {
            filtered_locales.push("_default");
        }

        // `intersectMaps` keeps the first map's values.
        let mut language_specific: Option<HashMap<u32, u32>> = None;
        for locale in filtered_locales {
            let map = array_to_map(lookup(locale).unwrap_or_default());
            language_specific = Some(match language_specific {
                None => map,
                Some(previous) => previous
                    .into_iter()
                    .filter(|(key, _)| map.contains_key(key))
                    .collect(),
            });
        }

        // `mergeMaps`: the language-specific values win.
        let mut confusable_dictionary = array_to_map(lookup("_common").unwrap_or_default());
        confusable_dictionary.extend(language_specific.unwrap_or_default());
        Self {
            confusable_dictionary,
        }
    }

    /// Monaco `getLocales`: the locales with their own data.
    pub fn get_locales() -> &'static [String] {
        &LOCALES
    }

    /// Monaco `isAmbiguous`.
    pub fn is_ambiguous(&self, code_point: u32) -> bool {
        self.confusable_dictionary.contains_key(&code_point)
    }

    /// Monaco `containsAmbiguousCharacter`.
    pub fn contains_ambiguous_character(&self, text: &str) -> bool {
        text.chars().any(|c| self.is_ambiguous(c as u32))
    }

    /// Monaco `getPrimaryConfusable`: the basic ASCII code point
    /// `code_point` can be confused with.
    pub fn get_primary_confusable(&self, code_point: u32) -> Option<u32> {
        self.confusable_dictionary.get(&code_point).copied()
    }

    /// Monaco `getConfusableCodePoints`.
    pub fn confusable_code_points(&self) -> impl Iterator<Item = u32> + '_ {
        self.confusable_dictionary.keys().copied()
    }
}

/// Characters that render as nothing or as whitespace (Monaco
/// `InvisibleCharacters`).
pub struct InvisibleCharacters;

impl InvisibleCharacters {
    /// Monaco `isInvisibleCharacter`.
    pub fn is_invisible_character(code_point: u32) -> bool {
        INVISIBLE_DATA.contains(&code_point)
    }

    /// Monaco `containsInvisibleCharacter`: an invisible character or a
    /// plain space.
    pub fn contains_invisible_character(text: &str) -> bool {
        text.chars()
            .any(|c| c == ' ' || Self::is_invisible_character(c as u32))
    }

    /// Monaco `codePoints`.
    pub fn code_points() -> &'static HashSet<u32> {
        &INVISIBLE_DATA
    }
}

/// Monaco `UnicodeHighlighterOptions`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnicodeHighlighterOptions {
    /// Highlight every character outside basic ASCII.
    pub non_basic_ascii: bool,
    /// Highlight characters confusable with basic ASCII ones.
    pub ambiguous_characters: bool,
    /// Highlight invisible characters.
    pub invisible_characters: bool,
    /// Keep highlights inside comment tokens. Not used by
    /// [`compute_unicode_highlights`]: like Monaco (whose decorations set
    /// `hideInCommentTokens: !includeComments`), the caller hides ranges
    /// over comment tokens when this is false.
    pub include_comments: bool,
    /// Keep highlights inside string tokens; like
    /// [`Self::include_comments`], applied by the caller
    /// (`hideInStringTokens`).
    pub include_strings: bool,
    /// Code points never highlighted.
    pub allowed_code_points: HashSet<u32>,
    /// Locales whose characters aren't ambiguous.
    pub allowed_locales: Vec<String>,
}

/// Monaco `IUnicodeHighlightsResult`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnicodeHighlighterResult {
    /// The highlighted characters, in order. At most [`MAX_RESULT_COUNT`].
    pub ranges: Vec<Range<PointUtf16>>,
    /// More characters would be highlighted past [`MAX_RESULT_COUNT`].
    pub has_more: bool,
    /// How many ambiguous characters were found.
    pub ambiguous_character_count: usize,
    /// How many invisible characters were found.
    pub invisible_character_count: usize,
    /// How many other non-basic-ASCII characters were found.
    pub non_basic_ascii_character_count: usize,
}

/// Monaco `UnicodeHighlighterReason`: why a character is highlighted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnicodeHighlighterReason {
    /// Confusable with a basic ASCII character.
    Ambiguous {
        /// The character it looks like.
        confusable_with: char,
        /// The locales in which it isn't ambiguous.
        not_ambiguous_in_locales: Vec<String>,
    },
    /// Invisible.
    Invisible,
    /// Outside basic ASCII.
    NonBasicAscii,
}

/// Monaco `computeUnicodeHighlights`: the characters to highlight in the
/// rows of `range` (all rows when `None`; columns are ignored, as in
/// Monaco).
///
/// Works on UTF-16 code units exactly like Monaco's regex search, quirks
/// included: a character outside the BMP is reported once per surrogate
/// (two identical ranges), and when it ends its line the high surrogate
/// alone is reported first.
pub fn compute_unicode_highlights(
    rope: &Rope,
    options: &UnicodeHighlighterOptions,
    range: Option<Range<PointUtf16>>,
) -> UnicodeHighlighterResult {
    let last_row = rope.line_count().saturating_sub(1);
    let (start_row, end_row) = match range {
        Some(range) => (range.start.row, range.end.row.min(last_row)),
        None => (0, last_row),
    };

    let highlighter = CodePointHighlighter::new(options);
    let candidates = highlighter.candidate_code_units();
    let mut result = UnicodeHighlighterResult::default();

    'rows: for row in start_row..=end_row {
        let line: Vec<u16> = rope.line(row).encode_utf16().collect();
        let line_len = line.len();
        for index in 0..line_len {
            if !candidates.matches(line[index]) {
                continue;
            }
            let mut start = index;
            let mut end = index + 1;
            // Extend the range to the entire code point.
            if start > 0 && is_high_surrogate(line[start - 1]) {
                start -= 1;
            }
            if end + 1 < line_len && is_high_surrogate(line[end - 1]) {
                end += 1;
            }
            let character = &line[start..end];
            // The word doesn't count unless it includes the character.
            let word = word_at_text(start + 1, &line, 0)
                .filter(|word| word.end > start)
                .map(|word| &line[word]);

            let count = match highlighter.should_highlight_non_basic_ascii(character, word) {
                SimpleHighlightReason::None => continue,
                SimpleHighlightReason::Ambiguous => &mut result.ambiguous_character_count,
                SimpleHighlightReason::Invisible => &mut result.invisible_character_count,
                SimpleHighlightReason::NonBasicAscii => &mut result.non_basic_ascii_character_count,
            };
            *count += 1;

            if result.ranges.len() >= MAX_RESULT_COUNT {
                result.has_more = true;
                break 'rows;
            }
            result
                .ranges
                .push(PointUtf16::new(row, start as u32)..PointUtf16::new(row, end as u32));
        }
    }
    result
}

/// Monaco `computeUnicodeHighlightReason`: why `character` (its first code
/// point) would be highlighted, ignoring its word.
pub fn compute_unicode_highlight_reason(
    character: &str,
    options: &UnicodeHighlighterOptions,
) -> Option<UnicodeHighlighterReason> {
    let highlighter = CodePointHighlighter::new(options);
    let units: Vec<u16> = character.encode_utf16().collect();
    match highlighter.should_highlight_non_basic_ascii(&units, None) {
        SimpleHighlightReason::None => None,
        SimpleHighlightReason::Invisible => Some(UnicodeHighlighterReason::Invisible),
        SimpleHighlightReason::NonBasicAscii => Some(UnicodeHighlighterReason::NonBasicAscii),
        SimpleHighlightReason::Ambiguous => {
            let code_point = code_point_at(&units, 0)?;
            let confusable_with = highlighter
                .ambiguous_characters
                .get_primary_confusable(code_point)
                .and_then(char::from_u32)?;
            let not_ambiguous_in_locales = AmbiguousCharacters::get_locales()
                .iter()
                .filter(|locale| {
                    // Monaco passes `new Set([...allowedLocales, l])`.
                    let mut locales: Vec<&str> = Vec::new();
                    for l in options
                        .allowed_locales
                        .iter()
                        .map(String::as_str)
                        .chain([locale.as_str()])
                    {
                        if !locales.contains(&l) {
                            locales.push(l);
                        }
                    }
                    !AmbiguousCharacters::get_instance(locales).is_ambiguous(code_point)
                })
                .cloned()
                .collect();
            Some(UnicodeHighlighterReason::Ambiguous {
                confusable_with,
                not_ambiguous_in_locales,
            })
        }
    }
}

/// Monaco `SimpleHighlightReason`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SimpleHighlightReason {
    None,
    NonBasicAscii,
    Invisible,
    Ambiguous,
}

/// The UTF-16 code units Monaco's search regex matches (it has no `u`
/// flag, so a character outside the BMP contributes both surrogates).
enum Candidates {
    /// `[^\t\n\r\x20-\x7E]`.
    AllNonBasicAscii,
    Units(HashSet<u16>),
}

impl Candidates {
    fn matches(&self, unit: u16) -> bool {
        match self {
            Self::AllNonBasicAscii => !is_basic_ascii(unit as u32),
            Self::Units(units) => units.contains(&unit),
        }
    }
}

/// Monaco `CodePointHighlighter`.
struct CodePointHighlighter<'a> {
    options: &'a UnicodeHighlighterOptions,
    ambiguous_characters: Arc<AmbiguousCharacters>,
}

impl<'a> CodePointHighlighter<'a> {
    fn new(options: &'a UnicodeHighlighterOptions) -> Self {
        let mut locales: Vec<&str> = Vec::new();
        for locale in &options.allowed_locales {
            if !locales.contains(&locale.as_str()) {
                locales.push(locale);
            }
        }
        Self {
            options,
            ambiguous_characters: AmbiguousCharacters::get_instance(locales),
        }
    }

    /// Monaco `getCandidateCodePoints`, as the code units its regex matches.
    fn candidate_code_units(&self) -> Candidates {
        if self.options.non_basic_ascii {
            return Candidates::AllNonBasicAscii;
        }
        let mut code_points = HashSet::new();
        if self.options.invisible_characters {
            code_points.extend(
                InvisibleCharacters::code_points()
                    .iter()
                    .copied()
                    .filter(|&cp| !is_allowed_invisible_character(&[cp])),
            );
        }
        if self.options.ambiguous_characters {
            code_points.extend(self.ambiguous_characters.confusable_code_points());
        }
        let mut units = HashSet::new();
        for cp in code_points {
            if self.options.allowed_code_points.contains(&cp) {
                continue;
            }
            if let Some(c) = char::from_u32(cp) {
                units.extend(c.encode_utf16(&mut [0; 2]).iter().copied());
            } else if cp <= 0xFFFF {
                units.insert(cp as u16);
            }
        }
        Candidates::Units(units)
    }

    /// Monaco `shouldHighlightNonBasicASCII`. `character` and `word` are
    /// UTF-16 code units.
    fn should_highlight_non_basic_ascii(
        &self,
        character: &[u16],
        word: Option<&[u16]>,
    ) -> SimpleHighlightReason {
        let code_point = code_point_at(character, 0);

        if code_point.is_some_and(|cp| self.options.allowed_code_points.contains(&cp)) {
            return SimpleHighlightReason::None;
        }

        if self.options.non_basic_ascii {
            return SimpleHighlightReason::NonBasicAscii;
        }
        let Some(code_point) = code_point else {
            return SimpleHighlightReason::None;
        };

        let mut has_basic_ascii_characters = false;
        let mut has_non_confusable_non_basic_ascii_character = false;
        if let Some(word) = word {
            for cp in code_points(word) {
                let is_basic_ascii = is_basic_ascii(cp);
                has_basic_ascii_characters |= is_basic_ascii;
                if !is_basic_ascii
                    && !self.ambiguous_characters.is_ambiguous(cp)
                    && !InvisibleCharacters::is_invisible_character(cp)
                {
                    has_non_confusable_non_basic_ascii_character = true;
                }
            }
        }

        // Don't allow mixing weird looking characters with ASCII, unless
        // there is an obviously weird looking character.
        if !has_basic_ascii_characters && has_non_confusable_non_basic_ascii_character {
            return SimpleHighlightReason::None;
        }

        if self.options.invisible_characters {
            let character: Vec<u32> = code_points(character).collect();
            if !is_allowed_invisible_character(&character)
                && InvisibleCharacters::is_invisible_character(code_point)
            {
                return SimpleHighlightReason::Invisible;
            }
        }

        if self.options.ambiguous_characters && self.ambiguous_characters.is_ambiguous(code_point) {
            return SimpleHighlightReason::Ambiguous;
        }

        SimpleHighlightReason::None
    }
}

/// Monaco `isAllowedInvisibleCharacter`, on a string's code points.
fn is_allowed_invisible_character(character: &[u32]) -> bool {
    matches!(character, [0x20 | 0x0A | 0x09])
}

/// Monaco `isBasicASCII` for one code point: `[\t\n\r\x20-\x7E]`.
fn is_basic_ascii(code_point: u32) -> bool {
    matches!(code_point, 0x09 | 0x0A | 0x0D | 0x20..=0x7E)
}

fn is_high_surrogate(unit: u16) -> bool {
    (0xD800..=0xDBFF).contains(&unit)
}

fn is_low_surrogate(unit: u16) -> bool {
    (0xDC00..=0xDFFF).contains(&unit)
}

/// JavaScript `String.prototype.codePointAt`.
fn code_point_at(units: &[u16], index: usize) -> Option<u32> {
    let first = *units.get(index)?;
    if is_high_surrogate(first)
        && let Some(&second) = units.get(index + 1)
        && is_low_surrogate(second)
    {
        return Some(0x10000 + ((first as u32 - 0xD800) << 10) + (second as u32 - 0xDC00));
    }
    Some(first as u32)
}

/// JavaScript string iteration: code points, lone surrogates as themselves.
fn code_points(units: &[u16]) -> impl Iterator<Item = u32> + '_ {
    let mut index = 0;
    std::iter::from_fn(move || {
        let cp = code_point_at(units, index)?;
        index += if cp > 0xFFFF { 2 } else { 1 };
        Some(cp)
    })
}

/// Monaco `USUAL_WORD_SEPARATORS`.
const USUAL_WORD_SEPARATORS: &str = "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?";

/// JavaScript regex `\s`.
fn is_js_whitespace(unit: u16) -> bool {
    matches!(
        unit,
        0x09..=0x0D
            | 0x20
            | 0xA0
            | 0x1680
            | 0x2000..=0x200A
            | 0x2028
            | 0x2029
            | 0x202F
            | 0x205F
            | 0x3000
            | 0xFEFF
    )
}

fn is_ascii_digit(unit: u16) -> bool {
    (b'0' as u16..=b'9' as u16).contains(&unit)
}

/// JavaScript regex `\w`.
fn is_word_unit(unit: u16) -> bool {
    u8::try_from(unit).is_ok_and(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Monaco `DEFAULT_WORD_REGEXP`, matched at `start`:
/// `(-?\d*\.\d\w*)|([^<separators>\s]+)`. Returns the match's end.
fn word_regexp_match_at(text: &[u16], start: usize) -> Option<usize> {
    let at = |i: usize| text.get(i).copied();

    // A number such as `-1.5e3`.
    let mut i = start;
    if at(i) == Some(b'-' as u16) {
        i += 1;
    }
    while at(i).is_some_and(is_ascii_digit) {
        i += 1;
    }
    if at(i) == Some(b'.' as u16) && at(i + 1).is_some_and(is_ascii_digit) {
        i += 2;
        while at(i).is_some_and(is_word_unit) {
            i += 1;
        }
        return Some(i);
    }

    let is_word = |unit: u16| {
        !is_js_whitespace(unit)
            && !(unit < 0x80 && USUAL_WORD_SEPARATORS.as_bytes().contains(&(unit as u8)))
    };
    let mut i = start;
    while at(i).is_some_and(is_word) {
        i += 1;
    }
    (i > start).then_some(i)
}

/// `RegExp.prototype.exec` with `lastIndex = from`.
fn word_regexp_exec(text: &[u16], from: usize) -> Option<Range<usize>> {
    (from..text.len()).find_map(|start| word_regexp_match_at(text, start).map(|end| start..end))
}

/// Monaco `_findRegexMatchEnclosingPosition`.
fn find_match_enclosing_position(
    text: &[u16],
    mut last_index: usize,
    pos: usize,
    stop_pos: isize,
) -> Option<Range<usize>> {
    while let Some(found) = word_regexp_exec(text, last_index) {
        last_index = found.end;
        if found.start <= pos && found.end >= pos {
            return Some(found);
        } else if stop_pos > 0 && found.start as isize > stop_pos {
            return None;
        }
    }
    None
}

/// Monaco's default `IGetWordAtTextConfig`.
const WORD_MAX_LEN: usize = 1000;
const WORD_WINDOW_SIZE: usize = 15;

/// Monaco `getWordAtText` with `DEFAULT_WORD_REGEXP` and the default
/// config, minus the time budget: the word around 1-based `column`, as a
/// 0-based code unit range of the original text.
fn word_at_text(column: usize, text: &[u16], text_offset: usize) -> Option<Range<usize>> {
    if text.len() > WORD_MAX_LEN {
        // Don't throw strings that long at the regexp, but use a
        // substring in which a word must occur.
        let start = column.saturating_sub(WORD_MAX_LEN / 2);
        let end = (column + WORD_MAX_LEN / 2).min(text.len());
        return word_at_text(column, &text[start..end], text_offset + start);
    }

    let pos = column - 1 - text_offset;
    let mut prev_regex_index: isize = -1;
    let mut found: Option<Range<usize>> = None;
    for i in 1.. {
        let regex_index = pos as isize - (WORD_WINDOW_SIZE * i) as isize;
        let this_match =
            find_match_enclosing_position(text, regex_index.max(0) as usize, pos, prev_regex_index);
        if this_match.is_none() && found.is_some() {
            break;
        }
        found = this_match;
        if regex_index <= 0 {
            break;
        }
        prev_regex_index = regex_index;
    }
    found.map(|range| range.start + text_offset..range.end + text_offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> UnicodeHighlighterOptions {
        UnicodeHighlighterOptions {
            ambiguous_characters: true,
            invisible_characters: true,
            ..Default::default()
        }
    }

    /// Ranges as Monaco's `Range.toString()`.
    fn monaco_ranges(result: &UnicodeHighlighterResult) -> Vec<String> {
        result
            .ranges
            .iter()
            .map(|range| {
                let (sl, sc) = range.start.to_monaco();
                let (el, ec) = range.end.to_monaco();
                format!("[{sl},{sc} -> {el},{ec}]")
            })
            .collect()
    }

    fn highlights(text: &str, options: &UnicodeHighlighterOptions) -> UnicodeHighlighterResult {
        compute_unicode_highlights(&Rope::from(text), options, None)
    }

    #[test]
    fn compute_unicode_highlights_168068() {
        let result = highlights("\n\tFor\u{a0}å\u{a0}gi\u{a0}et\u{a0}eksempel\n", &options());
        assert_eq!(result.ambiguous_character_count, 0);
        assert!(!result.has_more);
        assert_eq!(result.invisible_character_count, 4);
        assert_eq!(result.non_basic_ascii_character_count, 0);
        assert_eq!(
            monaco_ranges(&result),
            [
                "[2,5 -> 2,6]",
                "[2,7 -> 2,8]",
                "[2,10 -> 2,11]",
                "[2,13 -> 2,14]"
            ]
        );
    }

    #[test]
    fn cyrillic_a_is_ambiguous_by_default() {
        let instance = AmbiguousCharacters::get_instance(Vec::<String>::new());
        assert!(instance.is_ambiguous(0x0430));
        assert_eq!(instance.get_primary_confusable(0x0430), Some('a' as u32));
        assert!(!instance.is_ambiguous('a' as u32));
        assert!(instance.contains_ambiguous_character("p\u{430}ss"));
        assert!(!instance.contains_ambiguous_character("pass"));
        // Unknown locales fall back to `_default`.
        assert!(AmbiguousCharacters::get_instance(["xx"]).is_ambiguous(0x0430));
        // Russian text uses it.
        assert!(!AmbiguousCharacters::get_instance(["ru"]).is_ambiguous(0x0430));
    }

    #[test]
    fn locales() {
        let locales = AmbiguousCharacters::get_locales();
        assert_eq!(locales.first().map(String::as_str), Some("cs"));
        assert!(locales.iter().any(|l| l == "ru"));
        assert!(locales.iter().all(|l| !l.starts_with('_')));
    }

    #[test]
    fn invisible_characters() {
        assert!(InvisibleCharacters::is_invisible_character(0x200B));
        assert!(InvisibleCharacters::is_invisible_character(0xA0));
        assert!(!InvisibleCharacters::is_invisible_character(' ' as u32));
        assert!(!InvisibleCharacters::is_invisible_character('a' as u32));
        assert!(InvisibleCharacters::contains_invisible_character("a b"));
        assert!(InvisibleCharacters::contains_invisible_character(
            "a\u{200b}b"
        ));
        assert!(!InvisibleCharacters::contains_invisible_character("ab"));
    }

    #[test]
    fn ambiguous_mixed_with_ascii() {
        let result = highlights("let p\u{430}ss = 1;", &options());
        assert_eq!(result.ambiguous_character_count, 1);
        assert_eq!(monaco_ranges(&result), ["[1,6 -> 1,7]"]);

        let ru = UnicodeHighlighterOptions {
            allowed_locales: vec!["ru".into()],
            ..options()
        };
        assert_eq!(highlights("let p\u{430}ss = 1;", &ru).ranges, []);
    }

    #[test]
    fn non_ascii_word_with_obvious_letters_is_not_highlighted() {
        // `р` and `е` are confusable, but `п` makes the word obviously
        // not ASCII.
        assert_eq!(highlights("привет", &options()).ranges, []);
        // A word made only of confusables is highlighted.
        let result = highlights("ре", &options());
        assert_eq!(result.ambiguous_character_count, 2);
    }

    #[test]
    fn allowed_code_points() {
        let allowed = UnicodeHighlighterOptions {
            allowed_code_points: [0x430].into(),
            ..options()
        };
        assert_eq!(highlights("p\u{430}ss", &allowed).ranges, []);
    }

    #[test]
    fn non_basic_ascii() {
        let options = UnicodeHighlighterOptions {
            non_basic_ascii: true,
            ..Default::default()
        };
        let result = highlights("héllo\tworld", &options);
        assert_eq!(result.non_basic_ascii_character_count, 1);
        assert_eq!(monaco_ranges(&result), ["[1,2 -> 1,3]"]);

        // Monaco reports astral characters once per surrogate.
        let result = highlights("a😀b", &options);
        assert_eq!(monaco_ranges(&result), ["[1,2 -> 1,4]", "[1,2 -> 1,4]"]);
        let result = highlights("a😀", &options);
        assert_eq!(monaco_ranges(&result), ["[1,2 -> 1,3]", "[1,2 -> 1,4]"]);
    }

    #[test]
    fn max_result_count() {
        let text = "\u{200b}".repeat(MAX_RESULT_COUNT + 5);
        let result = highlights(&text, &options());
        assert!(result.has_more);
        assert_eq!(result.ranges.len(), MAX_RESULT_COUNT);
        assert_eq!(result.invisible_character_count, MAX_RESULT_COUNT + 1);
    }

    #[test]
    fn range_limits_rows() {
        let rope = Rope::from("\u{200b}\n\u{200b}\n\u{200b}");
        let result = compute_unicode_highlights(
            &rope,
            &options(),
            Some(PointUtf16::new(1, 0)..PointUtf16::new(1, 0)),
        );
        assert_eq!(monaco_ranges(&result), ["[2,1 -> 2,2]"]);
    }

    #[test]
    fn highlight_reason() {
        assert_eq!(
            compute_unicode_highlight_reason("\u{200b}", &options()),
            Some(UnicodeHighlighterReason::Invisible)
        );
        assert_eq!(compute_unicode_highlight_reason(" ", &options()), None);
        assert_eq!(compute_unicode_highlight_reason("a", &options()), None);
        assert_eq!(
            compute_unicode_highlight_reason(
                "é",
                &UnicodeHighlighterOptions {
                    non_basic_ascii: true,
                    ..Default::default()
                }
            ),
            Some(UnicodeHighlighterReason::NonBasicAscii)
        );
        assert_eq!(
            compute_unicode_highlight_reason("\u{430}", &options()),
            Some(UnicodeHighlighterReason::Ambiguous {
                confusable_with: 'a',
                not_ambiguous_in_locales: vec!["ru".into()],
            })
        );
    }

    #[test]
    fn word_at_text_matches_monaco() {
        let units = |s: &str| s.encode_utf16().collect::<Vec<_>>();
        assert_eq!(word_at_text(1, &units("foo bar"), 0), Some(0..3));
        assert_eq!(word_at_text(4, &units("foo bar"), 0), Some(0..3));
        assert_eq!(word_at_text(5, &units("foo bar"), 0), Some(4..7));
        assert_eq!(word_at_text(3, &units("x -1.5e3"), 0), Some(2..8));
        assert_eq!(word_at_text(1, &units("  "), 0), None);
    }
}
