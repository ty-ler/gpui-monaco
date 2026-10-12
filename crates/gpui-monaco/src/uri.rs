//! Monaco's uri normalization (`URI.parse(value).toString()`, in
//! `base/common/uri.ts`): models, markers and editor openers compare uris
//! by this form, so `file:///a b` and `file:///a%20b`, or `file:///C:/x`
//! and `file:///c%3A/x`, are the same document.

use std::fmt::Write as _;

/// A parsed uri's components, percent-decoded (Monaco `URI`).
struct Uri {
    scheme: String,
    authority: String,
    path: String,
    query: String,
    fragment: String,
}

/// `value` as Monaco's `URI.parse(value).toString()` writes it: the scheme
/// as given (`file` when missing), the authority lowercased, a Windows
/// drive letter lowercased, and every component percent-decoded and then
/// encoded the one way Monaco encodes it (`file:///a b` gives
/// `file:///a%20b`, `file:///C:/x` gives `file:///c%3A/x`, `?t=1` gives
/// `?t%3D1`). [`model_for_uri`](crate::model_for_uri), markers and the
/// [`TextModel::resource`](crate::TextModel::resource) use this form. A
/// value Monaco's parser rejects (an illegal scheme, or a path that
/// conflicts with the authority) is returned as given.
pub fn normalize_uri(value: &str) -> String {
    match parse(value) {
        Some(uri) => uri.to_string(),
        None => value.to_string(),
    }
}

/// Monaco `URI.parse` (not strict): splits `value` by `_regexp`, decodes
/// the components, then applies `_schemeFix`, `_referenceResolution` and
/// `_validateUri`.
fn parse(value: &str) -> Option<Uri> {
    // `/^(([^:/?#]+?):)?(\/\/([^/?#]*))?([^?#]*)(\?([^#]*))?(#(.*))?/`
    let mut rest = value;
    let mut scheme = "";
    if let Some(ix) = rest.find([':', '/', '?', '#'])
        && ix > 0
        && rest.as_bytes()[ix] == b':'
    {
        scheme = &rest[..ix];
        rest = &rest[ix + 1..];
    }
    let mut authority = "";
    if let Some(after) = rest.strip_prefix("//") {
        let end = after.find(['/', '?', '#']).unwrap_or(after.len());
        authority = &after[..end];
        rest = &after[end..];
    }
    let end = rest.find(['?', '#']).unwrap_or(rest.len());
    let path = &rest[..end];
    rest = &rest[end..];
    let mut query = "";
    if let Some(after) = rest.strip_prefix('?') {
        let end = after.find('#').unwrap_or(after.len());
        query = &after[..end];
        rest = &after[end..];
    }
    let mut fragment = "";
    if let Some(after) = rest.strip_prefix('#') {
        // JavaScript's `.` stops at line terminators.
        let end = after
            .find(['\n', '\r', '\u{2028}', '\u{2029}'])
            .unwrap_or(after.len());
        fragment = &after[..end];
    }

    let scheme = if scheme.is_empty() { "file" } else { scheme };
    let mut path = percent_decode(path);
    if matches!(scheme, "https" | "http" | "file") && !path.starts_with('/') {
        path.insert(0, '/');
    }
    let uri = Uri {
        scheme: scheme.to_string(),
        authority: percent_decode(authority),
        path,
        query: percent_decode(query),
        fragment: percent_decode(fragment),
    };
    // `_validateUri`: `^\w[\w\d+.-]*$`, and the path rules of RFC 3986 §3.3.
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut chars = uri.scheme.chars();
    if !chars.next().is_some_and(word) || !chars.all(|c| word(c) || "+.-".contains(c)) {
        return None;
    }
    if !uri.path.is_empty() {
        if !uri.authority.is_empty() && !uri.path.starts_with('/') {
            return None;
        }
        if uri.authority.is_empty() && uri.path.starts_with("//") {
            return None;
        }
    }
    Some(uri)
}

impl std::fmt::Display for Uri {
    /// Monaco `_asFormatted(uri, false)`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut res = String::new();
        let scheme = &self.scheme;
        if !scheme.is_empty() {
            res.push_str(scheme);
            res.push(':');
        }
        if !self.authority.is_empty() || scheme == "file" {
            res.push_str("//");
        }
        if !self.authority.is_empty() {
            let mut authority = self.authority.as_str();
            if let Some(ix) = authority.find('@') {
                let userinfo = &authority[..ix];
                authority = &authority[ix + 1..];
                match userinfo.rfind(':') {
                    None => encode(&mut res, userinfo, false, false),
                    Some(ix) => {
                        encode(&mut res, &userinfo[..ix], false, false);
                        res.push(':');
                        encode(&mut res, &userinfo[ix + 1..], false, true);
                    }
                }
                res.push('@');
            }
            let authority = authority.to_lowercase();
            match authority.rfind(':') {
                None => encode(&mut res, &authority, false, true),
                Some(ix) => {
                    encode(&mut res, &authority[..ix], false, true);
                    res.push_str(&authority[ix..]);
                }
            }
        }
        if !self.path.is_empty() {
            // HTTP paths are case-sensitive, even when their first segment
            // resembles a drive letter.
            let web = matches!(scheme.to_lowercase().as_str(), "http" | "https");
            let bytes = self.path.as_bytes();
            let mut path = self.path.clone();
            if bytes.len() >= 3 && bytes[0] == b'/' && bytes[2] == b':' {
                if bytes[1].is_ascii_uppercase() && !web {
                    path = format!("/{}:{}", bytes[1].to_ascii_lowercase() as char, &path[3..]);
                }
            } else if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_uppercase() && !web
            {
                path = format!("{}:{}", bytes[0].to_ascii_lowercase() as char, &path[2..]);
            }
            encode(&mut res, &path, true, false);
        }
        if !self.query.is_empty() {
            res.push('?');
            encode(&mut res, &self.query, false, false);
        }
        if !self.fragment.is_empty() {
            res.push('#');
            encode(&mut res, &self.fragment, false, false);
        }
        f.write_str(&res)
    }
}

/// Monaco `encodeTable`: the reserved characters of RFC 3986 §2.2, and the
/// space.
fn reserved_escape(c: char) -> Option<&'static str> {
    Some(match c {
        ':' => "%3A",
        '/' => "%2F",
        '?' => "%3F",
        '#' => "%23",
        '[' => "%5B",
        ']' => "%5D",
        '@' => "%40",
        '!' => "%21",
        '$' => "%24",
        '&' => "%26",
        '\'' => "%27",
        '(' => "%28",
        ')' => "%29",
        '*' => "%2A",
        '+' => "%2B",
        ',' => "%2C",
        ';' => "%3B",
        '=' => "%3D",
        ' ' => "%20",
        _ => return None,
    })
}

/// Monaco `encodeURIComponentFast`: unreserved characters stay (and `/` in
/// a path, `[`, `]` and `:` in an authority), reserved ones use
/// [`reserved_escape`], and the rest are UTF-8 percent-encoded as
/// `encodeURIComponent` does.
fn encode(out: &mut String, component: &str, is_path: bool, is_authority: bool) {
    for c in component.chars() {
        let keep = c.is_ascii_alphanumeric()
            || matches!(c, '-' | '.' | '_' | '~')
            || (is_path && c == '/')
            || (is_authority && matches!(c, '[' | ']' | ':'));
        if keep {
            out.push(c);
        } else if let Some(escaped) = reserved_escape(c) {
            out.push_str(escaped);
        } else {
            let mut buf = [0; 4];
            for byte in c.encode_utf8(&mut buf).bytes() {
                write!(out, "%{byte:02X}").ok();
            }
        }
    }
}

/// Monaco `percentDecode`: each run of `%XX` (`XX` letters or digits) is
/// decoded as `decodeURIComponent` would, keeping the escapes it can't
/// decode (`decodeURIComponentGraceful`).
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let is_escape = |i: usize| {
        bytes[i] == b'%'
            && bytes.get(i + 1).is_some_and(u8::is_ascii_alphanumeric)
            && bytes.get(i + 2).is_some_and(u8::is_ascii_alphanumeric)
    };
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut copied = 0;
    while i < bytes.len() {
        if !is_escape(i) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && is_escape(i) {
            i += 3;
        }
        out.push_str(&text[copied..start]);
        decode_graceful(&mut out, &text[start..i]);
        copied = i;
    }
    out.push_str(&text[copied..]);
    out
}

/// Monaco `decodeURIComponentGraceful` for a run of `%XX` escapes: decodes
/// it, or keeps the first escape and tries the rest.
fn decode_graceful(out: &mut String, run: &str) {
    let mut run = run;
    loop {
        let decoded = run
            .as_bytes()
            .chunks(3)
            .map(|escape| {
                std::str::from_utf8(&escape[1..])
                    .ok()
                    .and_then(|hex| u8::from_str_radix(hex, 16).ok())
            })
            .collect::<Option<Vec<u8>>>()
            .and_then(|bytes| String::from_utf8(bytes).ok());
        if let Some(decoded) = decoded {
            out.push_str(&decoded);
            return;
        }
        if run.len() <= 3 {
            out.push_str(run);
            return;
        }
        out.push_str(&run[..3]);
        run = &run[3..];
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_uri;

    /// Monaco `uri.test.ts`: `URI.parse(value).toString()`.
    #[test]
    fn parse_then_to_string() {
        let cases = [
            ("file:///c:/test/me", "file:///c%3A/test/me"),
            ("file:///C:/test/me", "file:///c%3A/test/me"),
            (
                "file://shares/files/c%23/p.cs",
                "file://shares/files/c%23/p.cs",
            ),
            (
                "file:///c:/Source/Z%C3%BCrich%20or%20Zurich%20(%CB%88zj%CA%8A%C9%99r%C9%AAk,/Code/resources/app/plugins/c%23/plugin.json",
                "file:///c%3A/Source/Z%C3%BCrich%20or%20Zurich%20%28%CB%88zj%CA%8A%C9%99r%C9%AAk%2C/Code/resources/app/plugins/c%23/plugin.json",
            ),
            (
                "file:///c:/test with %25/c#code",
                "file:///c%3A/test%20with%20%25/c#code",
            ),
            (
                "file://shares/pröjects/c%23/#l12",
                "file://shares/pr%C3%B6jects/c%23/#l12",
            ),
            (
                "http://api/files/test.me?t=1234",
                "http://api/files/test.me?t%3D1234",
            ),
            (
                "http:/api/files/test.me?t=1234",
                "http:/api/files/test.me?t%3D1234",
            ),
            ("http://localhost:8080/far", "http://localhost:8080/far"),
            ("http://Localhost/C:/far", "http://localhost/C%3A/far"),
            ("foo:bar/path", "foo:bar/path"),
            ("inmemory://model/1", "inmemory://model/1"),
            (
                "untitled:c:/Users/x/abc.txt",
                "untitled:c%3A/Users/x/abc.txt",
            ),
            ("mailto:test@example.com", "mailto:test%40example.com"),
            (
                "http://a-test-site.com/#test=true",
                "http://a-test-site.com/#test%3Dtrue",
            ),
            ("file:?q", "file:///?q"),
            ("http://example.com", "http://example.com/"),
            ("/users/me/a b.txt", "file:///users/me/a%20b.txt"),
            // Escapes that don't decode stay, their `%` encoded.
            ("file://some/%.txt", "file://some/%25.txt"),
            ("file://some/%A0.txt", "file://some/%25A0.txt"),
            ("file:///a%2fb", "file:///a/b"),
        ];
        for (value, expected) in cases {
            assert_eq!(normalize_uri(value), expected, "{value}");
            // Normalizing is idempotent.
            assert_eq!(normalize_uri(expected), expected, "{expected}");
        }
    }

    #[test]
    fn spellings_of_one_uri() {
        assert_eq!(normalize_uri("file:///a b"), normalize_uri("file:///a%20b"));
        assert_eq!(
            normalize_uri("file:///C:/x.rs"),
            normalize_uri("file:///c%3A/x.rs")
        );
        assert_eq!(
            normalize_uri("HTTP://Example.COM/A"),
            "HTTP://example.com/A"
        );
    }

    /// Values `URI.parse` rejects are kept as given.
    #[test]
    fn invalid_uris_are_kept() {
        assert_eq!(normalize_uri("a b:c"), "a b:c");
        assert_eq!(normalize_uri("file:////x"), "file:////x");
    }
}
