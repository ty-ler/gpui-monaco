//! Translating JavaScript regular expression sources, as Monarch grammars
//! write them, into [`fancy_regex`] syntax.
//!
//! JavaScript semantics kept:
//! - `\d`, `\w` (and `\D`, `\W`) are ASCII-only, `\s` is JavaScript's
//!   whitespace set (with U+FEFF, without U+0085), and `.` matches anything
//!   but `\n`, `\r`, U+2028 and U+2029.
//! - `{` that doesn't start a quantifier (`{2}`, `{2,}`, `{2,5}`) and a lone
//!   `}` are literals, `[` inside a class is a literal, `[]` matches nothing
//!   and `[^]` anything.
//! - Identity escapes (`\"`, `\/`, `\-`, `\<`, ...) are literals, `\0` is
//!   NUL, `\cX` a control character, `\uXXXX` (and surrogate pairs) and
//!   `\u{X}` code points, and `\b` inside a class a backspace.
//!
//! Differences (documented on [`MonarchTokenizer`](super::MonarchTokenizer)):
//! `\b` and `\B` use Unicode word characters, matching works on code points
//! rather than UTF-16 code units, and case folding is Unicode's.

/// JavaScript's `\s`.
const JS_SPACE: &str =
    r"\t\n\x0B\x0C\r \xA0\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}";
const JS_WORD: &str = "0-9A-Za-z_";
const JS_DIGIT: &str = "0-9";

/// Characters that are special to `fancy_regex` outside or inside a class
/// and need a backslash to be literal.
fn is_meta(c: char) -> bool {
    matches!(
        c,
        '\\' | '.'
            | '+'
            | '*'
            | '?'
            | '('
            | ')'
            | '|'
            | '['
            | ']'
            | '{'
            | '}'
            | '^'
            | '$'
            | '#'
            | '&'
            | '-'
            | '~'
    )
}

fn push_literal(out: &mut String, c: char) {
    if is_meta(c) {
        out.push('\\');
        out.push(c);
    } else if c.is_whitespace() || c.is_control() {
        out.push_str(&format!("\\x{{{:X}}}", c as u32));
    } else {
        out.push(c);
    }
}

/// Translates the JavaScript regex `source` into `fancy_regex` syntax,
/// with `(?i)` for `ignore_case`.
pub(crate) fn translate(source: &str, ignore_case: bool) -> Result<String, String> {
    translate_with(source, ignore_case, false)
}

/// [`translate`], where `dot_all` (JavaScript's `s` flag) makes `.` match
/// any character.
pub(crate) fn translate_with(
    source: &str,
    ignore_case: bool,
    dot_all: bool,
) -> Result<String, String> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len() + 8);
    if ignore_case {
        out.push_str("(?i)");
    }
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' => {
                i = translate_escape(&chars, i, false, &mut out)?;
                continue;
            }
            '[' => {
                i = translate_class(&chars, i, &mut out)?;
                continue;
            }
            '.' if dot_all => out.push_str(r"[\x00-\x{10FFFF}]"),
            '.' => out.push_str(r"[^\n\r\x{2028}\x{2029}]"),
            '{' => match quantifier_len(&chars[i..]) {
                Some(len) => {
                    out.extend(&chars[i..i + len]);
                    i += len;
                    continue;
                }
                None => out.push_str(r"\{"),
            },
            '}' => out.push_str(r"\}"),
            '(' => {
                // Groups and lookarounds are the same; JavaScript's named
                // groups `(?<name>` are accepted by fancy_regex.
                out.push('(');
            }
            '#' | '&' | '~' => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_whitespace() => push_literal(&mut out, c),
            c => out.push(c),
        }
        i += 1;
    }
    Ok(out)
}

/// The length of a JavaScript quantifier `{n}`, `{n,}` or `{n,m}` at the
/// start of `chars`, if it is one.
fn quantifier_len(chars: &[char]) -> Option<usize> {
    let mut i = 1;
    let digits = |i: &mut usize| {
        let start = *i;
        while chars.get(*i).is_some_and(char::is_ascii_digit) {
            *i += 1;
        }
        *i > start
    };
    if !digits(&mut i) {
        return None;
    }
    if chars.get(i) == Some(&',') {
        i += 1;
        digits(&mut i);
    }
    (chars.get(i) == Some(&'}')).then_some(i + 1)
}

fn hex_value(chars: &[char]) -> Option<u32> {
    let s: String = chars.iter().collect();
    u32::from_str_radix(&s, 16).ok()
}

fn is_hex(chars: &[char]) -> bool {
    chars.iter().all(char::is_ascii_hexdigit)
}

/// A `\uXXXX` escape at `i` (pointing at the backslash), as a code unit.
fn u_escape(chars: &[char], i: usize) -> Option<u32> {
    let digits = chars.get(i + 2..i + 6)?;
    (chars[i + 1] == 'u' && is_hex(digits))
        .then(|| hex_value(digits))
        .flatten()
}

fn push_code_point(out: &mut String, code_point: u32) -> Result<(), String> {
    let c = char::from_u32(code_point)
        .ok_or_else(|| format!("invalid code point \\u{{{code_point:X}}}"))?;
    out.push_str(&format!("\\x{{{:X}}}", c as u32));
    Ok(())
}

/// Translates the escape at `chars[i]` (a backslash), returning the index
/// after it.
fn translate_escape(
    chars: &[char],
    i: usize,
    in_class: bool,
    out: &mut String,
) -> Result<usize, String> {
    let Some(&c) = chars.get(i + 1) else {
        return Err("\\ at end of pattern".into());
    };
    let mut end = i + 2;
    match c {
        'd' | 'w' | 's' => {
            let set = match c {
                'd' => JS_DIGIT,
                'w' => JS_WORD,
                _ => JS_SPACE,
            };
            if in_class {
                out.push_str(set);
            } else {
                out.push('[');
                out.push_str(set);
                out.push(']');
            }
        }
        'D' | 'W' | 'S' => {
            let set = match c {
                'D' => JS_DIGIT,
                'W' => JS_WORD,
                _ => JS_SPACE,
            };
            // A nested class when inside one.
            out.push_str("[^");
            out.push_str(set);
            out.push(']');
        }
        'b' if in_class => out.push_str(r"\x08"),
        'b' | 'B' => {
            out.push('\\');
            out.push(c);
        }
        't' | 'n' | 'r' | 'f' | 'v' => {
            out.push('\\');
            out.push(c);
        }
        '0' if !chars.get(end).is_some_and(char::is_ascii_digit) => out.push_str(r"\x00"),
        '1'..='9' if !in_class => {
            // A backreference.
            out.push('\\');
            out.push(c);
            while let Some(&d) = chars.get(end).filter(|d| d.is_ascii_digit()) {
                out.push(d);
                end += 1;
            }
        }
        'c' if chars.get(end).is_some_and(char::is_ascii_alphabetic) => {
            let letter = chars[end] as u32;
            push_code_point(out, letter % 32)?;
            end += 1;
        }
        'x' if chars.get(end..end + 2).is_some_and(is_hex) => {
            push_code_point(out, hex_value(&chars[end..end + 2]).unwrap_or(0))?;
            end += 2;
        }
        'u' if chars.get(end) == Some(&'{') => {
            let close = chars[end..]
                .iter()
                .position(|&c| c == '}')
                .map(|p| end + p)
                .ok_or("unterminated \\u{")?;
            let digits = &chars[end + 1..close];
            if digits.is_empty() || !is_hex(digits) {
                return Err("invalid \\u{...} escape".into());
            }
            push_code_point(out, hex_value(digits).ok_or("invalid \\u{...} escape")?)?;
            end = close + 1;
        }
        'u' if u_escape(chars, i).is_some() => {
            let unit = u_escape(chars, i).unwrap_or(0);
            end = i + 6;
            if (0xD800..0xDC00).contains(&unit)
                && chars.get(end) == Some(&'\\')
                && let Some(low) = u_escape(chars, end).filter(|u| (0xDC00..0xE000).contains(u))
            {
                push_code_point(out, 0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00))?;
                end += 6;
            } else if (0xD800..0xE000).contains(&unit) {
                // A lone surrogate can't occur in Rust strings.
                out.push_str(r"\x{FFFD}");
            } else {
                push_code_point(out, unit)?;
            }
        }
        'k' if !in_class && chars.get(end) == Some(&'<') => {
            out.push_str(r"\k");
        }
        'p' | 'P' if chars.get(end) == Some(&'{') => {
            let close = chars[end..]
                .iter()
                .position(|&c| c == '}')
                .map(|p| end + p)
                .ok_or("unterminated \\p{")?;
            out.push('\\');
            out.push(c);
            out.extend(&chars[end..=close]);
            end = close + 1;
        }
        // Identity escapes: the character itself.
        c => push_literal(out, c),
    }
    Ok(end)
}

/// Translates the class starting at `chars[i]` (a `[`), returning the
/// index after its `]`.
fn translate_class(chars: &[char], i: usize, out: &mut String) -> Result<usize, String> {
    let mut j = i + 1;
    let negated = chars.get(j) == Some(&'^');
    if negated {
        j += 1;
    }
    if chars.get(j) == Some(&']') {
        // `[]` matches nothing, `[^]` anything.
        out.push_str(if negated {
            r"[\x00-\x{10FFFF}]"
        } else {
            r"[^\x00-\x{10FFFF}]"
        });
        return Ok(j + 1);
    }
    out.push('[');
    if negated {
        out.push('^');
    }
    let mut previous_dash = false;
    // Whether the previous atom was a class escape such as `\w`.
    let mut previous_class_escape = false;
    loop {
        let Some(&c) = chars.get(j) else {
            return Err("unterminated character class".into());
        };
        match c {
            ']' => {
                out.push(']');
                return Ok(j + 1);
            }
            '\\' => {
                previous_class_escape = is_class_escape(chars, j);
                j = translate_escape(chars, j, true, out)?;
                previous_dash = false;
                continue;
            }
            // JavaScript (Annex B) reads a hyphen next to a class escape,
            // as in `[\w-.]`, as a literal; fancy_regex would see a range.
            '-' if previous_class_escape || is_class_escape(chars, j + 1) => {
                out.push_str(r"\-");
            }
            // `[`, `&&` and `~~` are special in fancy_regex classes, and
            // `--` is a difference.
            '[' | '&' | '~' => {
                out.push('\\');
                out.push(c);
            }
            '-' if previous_dash => out.push_str(r"\-"),
            c if c.is_whitespace() || c.is_control() => push_literal(out, c),
            c => out.push(c),
        }
        previous_dash = c == '-';
        previous_class_escape = false;
        j += 1;
    }
}

/// Whether `chars[i..]` starts with a class escape (`\d`, `\w`, `\s` or
/// their negations).
fn is_class_escape(chars: &[char], i: usize) -> bool {
    chars.get(i) == Some(&'\\')
        && matches!(chars.get(i + 1), Some('d' | 'D' | 'w' | 'W' | 's' | 'S'))
}

#[cfg(test)]
mod tests {
    use super::translate;

    fn matches(js: &str, text: &str) -> Option<String> {
        let translated = translate(js, false).unwrap();
        let regex = fancy_regex::Regex::new(&translated)
            .unwrap_or_else(|e| panic!("{js} -> {translated}: {e}"));
        regex.find(text).unwrap().map(|m| m.as_str().to_string())
    }

    #[test]
    fn javascript_classes_are_ascii() {
        assert_eq!(matches(r"\w+", "éab1_c"), Some("ab1_c".into()));
        assert_eq!(matches(r"\d+", "٣12"), Some("12".into()));
        assert_eq!(matches(r"[\w.]+", "é.a.b"), Some(".a.b".into()));
        assert_eq!(matches(r"[^\W]+", "é ab"), Some("ab".into()));
        assert_eq!(matches(r"\s", "a\u{feff}b"), Some("\u{feff}".into()));
        assert_eq!(matches(r"\s", "a\u{85}b"), None);
        assert_eq!(matches(r".+", "ab\rc"), Some("ab".into()));
    }

    #[test]
    fn braces_and_brackets_are_literal_where_javascript_says() {
        assert_eq!(matches(r"{", "a{b"), Some("{".into()));
        assert_eq!(matches(r"a{2}", "aaa"), Some("aa".into()));
        assert_eq!(matches(r"a{,2}", "a{,2}"), Some("a{,2}".into()));
        assert_eq!(matches(r"}", "}"), Some("}".into()));
        assert_eq!(matches(r"[[\]]+", "x[]y"), Some("[]".into()));
        assert_eq!(matches(r"[a&&b]+", "&&"), Some("&&".into()));
        assert_eq!(matches(r"[+--]+", "+,-"), Some("+,-".into()));
        assert_eq!(matches(r"[]", "abc"), None);
        assert_eq!(matches(r"[^]+", "a\nb"), Some("a\nb".into()));
    }

    #[test]
    fn escapes() {
        assert_eq!(
            matches(r#"\"\/\<\@\:"#, r#""/<@:"#),
            Some(r#""/<@:"#.into())
        );
        assert_eq!(matches(r"A\x42", "AB"), Some("AB".into()));
        assert_eq!(matches(r"😀", "😀"), Some("😀".into()));
        assert_eq!(matches(r"\u{1F600}", "😀"), Some("😀".into()));
        assert_eq!(matches(r"\cJ", "\n"), Some("\n".into()));
        assert_eq!(matches(r"\0", "\0"), Some("\0".into()));
        assert_eq!(matches(r"[\b]", "\u{8}"), Some("\u{8}".into()));
        assert_eq!(matches(r"(a)\1", "aa"), Some("aa".into()));
        assert_eq!(matches(r"(?<x>a)\k<x>", "aa"), Some("aa".into()));
        assert_eq!(matches(r"(?<=a)b", "ab"), Some("b".into()));
        assert_eq!(matches(r"\p{Lu}+", "aBC"), Some("BC".into()));
        assert_eq!(matches(r"a b#", "a b#"), Some("a b#".into()));
    }

    #[test]
    fn hyphens_next_to_class_escapes_are_literal() {
        assert_eq!(matches(r"[\w-.]+", "a-b.c é"), Some("a-b.c".into()));
        assert_eq!(matches(r"[.-\d]+", "x-.1"), Some("-.1".into()));
        assert_eq!(matches(r"[\W-z]+", "a z-!"), Some(" z-!".into()));
        assert_eq!(matches(r"[a-z]+", "-ab"), Some("ab".into()));
    }

    #[test]
    fn dot_all() {
        let dot_all = super::translate_with("a.b", false, true).unwrap();
        assert!(
            fancy_regex::Regex::new(&dot_all)
                .unwrap()
                .is_match("a\rb")
                .unwrap()
        );
        assert_eq!(matches("a.b", "a\rb"), None);
    }

    #[test]
    fn ignore_case() {
        let regex = fancy_regex::Regex::new(&translate("select", true).unwrap()).unwrap();
        assert!(regex.is_match("SeLeCt").unwrap());
    }
}
