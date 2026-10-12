//! `colorize` and `tokenize` without an editor (Monaco `colorizer.ts`).

use gpui_monaco::Theme;
use gpui_monaco::colorize::{ColorizeOptions, colorize, tokenize};

#[test]
fn tokenize_rust() {
    let registry = gpui_monaco_languages::registry();
    let lines = tokenize("fn main() {\n\tlet x = 1;\n}", "rust", &registry);
    assert_eq!(lines.len(), 3);
    // Tokens cover each line from the start, in order.
    for line in &lines {
        assert_eq!(line[0].offset, 0);
        assert!(line.windows(2).all(|w| w[0].offset < w[1].offset));
        assert!(line.iter().all(|t| t.language.as_ref() == "rust"));
    }
    assert_eq!(lines[0][0].token_type.as_ref(), "keyword.rust");
    // `\t` then `let` (offset 1).
    assert_eq!(lines[1][0].token_type.as_ref(), "");
    assert_eq!(lines[1][1].offset, 1);
    assert_eq!(lines[1][1].token_type.as_ref(), "keyword.rust");
    // Punctuation is a delimiter.
    let semicolon = lines[1].iter().find(|t| t.offset == 10).unwrap();
    assert_eq!(semicolon.token_type.as_ref(), "delimiter.rust");
}

#[test]
fn colorize_rust() {
    let registry = gpui_monaco_languages::registry();
    let theme = Theme::vs_dark();
    let lines = colorize(
        "fn main() {\n\tlet x = 1;\n}",
        "rust",
        &theme,
        &registry,
        ColorizeOptions { tab_size: 2 },
    );
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[1].text.as_ref(), "  let x = 1;");
    // `let` is a keyword color, after the expanded tab.
    let (range, style) = &lines[1].highlights[0];
    assert_eq!(*range, 2..5);
    assert!(style.color.is_some());
}

/// Monaco `colorizeElement` takes a MIME type too
/// (`getLanguageIdByMimeType`), including the `text/x-<id>` of languages
/// registered without one.
#[test]
fn colorize_by_mime_type() {
    let registry = gpui_monaco_languages::registry();
    let theme = Theme::vs_dark();
    let text = "fn main() {}";
    let options = ColorizeOptions::default();
    let by_id = colorize(text, "rust", &theme, &registry, options);
    let by_mime = colorize(text, "text/x-rust", &theme, &registry, options);
    assert!(!by_id[0].highlights.is_empty());
    assert_eq!(by_mime, by_id);
}
