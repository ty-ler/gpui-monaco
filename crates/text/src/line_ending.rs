use std::borrow::Cow;

/// A model's end-of-line sequence. Text is always stored with `\n`; the line
/// ending is applied when text is exported.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LineEnding {
    /// `\n` (Unix).
    #[default]
    Lf,
    /// `\r\n` (Windows).
    Crlf,
}

impl LineEnding {
    /// The line break sequence: `"\n"` or `"\r\n"`.
    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::Crlf => "\r\n",
        }
    }

    /// Monaco's rule (`PieceTreeTextBufferBuilder.getEOL`): CRLF when more than
    /// half of the line breaks contain a `\r`, else LF; `default` when the text
    /// has no line breaks.
    pub fn detect(text: &str, default: LineEnding) -> LineEnding {
        let bytes = text.as_bytes();
        let (mut cr, mut lf, mut crlf) = (0usize, 0usize, 0usize);
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'\r' if bytes.get(i + 1) == Some(&b'\n') => {
                    crlf += 1;
                    i += 1;
                }
                b'\r' => cr += 1,
                b'\n' => lf += 1,
                _ => {}
            }
            i += 1;
        }
        let total = cr + lf + crlf;
        if total == 0 {
            default
        } else if 2 * (cr + crlf) > total {
            LineEnding::Crlf
        } else {
            LineEnding::Lf
        }
    }
}

/// Replaces `\r\n` and lone `\r` with `\n`.
pub fn normalize_line_endings(text: &str) -> Cow<'_, str> {
    if !text.contains('\r') {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(ch);
        }
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_like_monaco() {
        assert_eq!(
            LineEnding::detect("abc", LineEnding::Crlf),
            LineEnding::Crlf
        );
        assert_eq!(LineEnding::detect("a\nb", LineEnding::Crlf), LineEnding::Lf);
        assert_eq!(
            LineEnding::detect("a\r\nb", LineEnding::Lf),
            LineEnding::Crlf
        );
        assert_eq!(
            LineEnding::detect("a\r\nb\nc\nd", LineEnding::Lf),
            LineEnding::Lf
        );
        assert_eq!(
            LineEnding::detect("a\r\nb\r\nc\nd", LineEnding::Lf),
            LineEnding::Crlf
        );
        assert_eq!(LineEnding::detect("a\rb", LineEnding::Lf), LineEnding::Crlf);
    }

    #[test]
    fn normalizes() {
        assert_eq!(normalize_line_endings("a\r\nb\rc\n"), "a\nb\nc\n");
        assert!(matches!(
            normalize_line_endings("plain\n"),
            Cow::Borrowed(_)
        ));
    }
}
