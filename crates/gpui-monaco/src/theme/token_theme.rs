//! Token rule resolution: Monaco's `TokenTheme` trie (MIT), from
//! `common/languages/supports/tokenization.ts`.
//!
//! Rules are sorted by selector and inserted into a trie of dot-separated
//! segments. A new node starts as a copy of its parent's rule, and a rule
//! only overwrites the fields it sets, so `comment.doc` with just a font
//! style keeps `comment`'s color. A token matches the deepest node along
//! its segments. `''` rules set the root, the default for every token.
//!
//! Deviations from Monaco:
//! - The root's color defaults to none (the editor foreground) instead of
//!   `#000000`, for VS Code themes. `Theme::define` gives a theme without
//!   `inherit` a `#000000` `''` rule first (an `editor.foreground` one
//!   otherwise), so standalone themes resolve as Monaco's do.
//! - Token backgrounds are not kept: Monaco's editor doesn't draw them.

use std::collections::HashMap;

use gpui::Hsla;

use super::{TokenFontStyle, TokenStyle};

/// A resolved rule (Monaco `ThemeTrieElementRule`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct TrieRule {
    /// `None` is the editor foreground.
    pub(crate) foreground: Option<Hsla>,
    pub(crate) font_style: TokenFontStyle,
}

impl TrieRule {
    /// Monaco `ThemeTrieElementRule.acceptOverwrite`: only set fields.
    fn accept_overwrite(&mut self, foreground: Option<Hsla>, font_style: Option<TokenFontStyle>) {
        if let Some(font_style) = font_style {
            self.font_style = font_style;
        }
        if let Some(foreground) = foreground {
            self.foreground = Some(foreground);
        }
    }

    pub(crate) fn style(self) -> TokenStyle {
        TokenStyle {
            foreground: self.foreground,
            bold: self.font_style.bold,
            italic: self.font_style.italic,
            underline: self.font_style.underline,
            strikethrough: self.font_style.strikethrough,
            opacity: None,
        }
    }
}

/// Monaco `ThemeTrieElement`.
#[derive(Clone, Debug, Default, PartialEq)]
struct Node {
    rule: TrieRule,
    /// Whether a rule was inserted at this node (rather than it only being
    /// on the way to a deeper one).
    explicit: bool,
    children: HashMap<String, Node>,
}

impl Node {
    fn insert(
        &mut self,
        token: &str,
        foreground: Option<Hsla>,
        font_style: Option<TokenFontStyle>,
    ) {
        if token.is_empty() {
            self.rule.accept_overwrite(foreground, font_style);
            self.explicit = true;
            return;
        }
        let (head, tail) = token.split_once('.').unwrap_or((token, ""));
        let rule = self.rule;
        self.children
            .entry(head.to_string())
            .or_insert_with(|| Node {
                rule,
                ..Node::default()
            })
            .insert(tail, foreground, font_style);
    }
}

/// A theme's rules of one vocabulary, resolved (Monaco `TokenTheme`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct TokenTrie {
    root: Node,
}

impl TokenTrie {
    /// Resolves `(selector, foreground, font style)` rules given in theme
    /// order; a `None` field is unset (Monaco `resolveParsedTokenThemeRules`).
    pub(crate) fn new<'a>(
        rules: impl IntoIterator<Item = (&'a str, Option<Hsla>, Option<TokenFontStyle>)>,
    ) -> Self {
        let mut rules: Vec<_> = rules.into_iter().enumerate().collect();
        // Monaco sorts with `strcmp`, then by index: parents come before
        // their children, and equal selectors merge in order.
        rules.sort_by(|(i, a), (j, b)| a.0.cmp(b.0).then(i.cmp(j)));
        let mut root = Node::default();
        for (_, (selector, foreground, font_style)) in rules {
            root.insert(selector, foreground, font_style);
        }
        Self { root }
    }

    /// The rule for `token` (Monaco `TokenTheme._match`), and whether a rule
    /// other than a `''` one applies.
    pub(crate) fn match_token(&self, token: &str) -> (TrieRule, bool) {
        let mut node = &self.root;
        let mut matched = false;
        let mut rest = token;
        while !rest.is_empty() {
            let (head, tail) = rest.split_once('.').unwrap_or((rest, ""));
            let Some(child) = node.children.get(head) else {
                break;
            };
            matched |= child.explicit;
            node = child;
            rest = tail;
        }
        (node.rule, matched)
    }

    /// The style of the most specific rule matching `token`, if any but a
    /// `''` rule does.
    pub(crate) fn specific(&self, token: &str) -> Option<TokenStyle> {
        let (rule, matched) = self.match_token(token);
        matched.then(|| rule.style())
    }

    /// The `''` rules' style, if there are any.
    pub(crate) fn root(&self) -> Option<TokenStyle> {
        self.root.explicit.then(|| self.root.rule.style())
    }
}

#[cfg(test)]
mod tests {
    //! Ported from Monaco's `tokenization.test.ts`. Backgrounds are left
    //! out (see the module docs).

    use super::*;
    use crate::theme::{TokenFontStyle, parse_token_color};

    const NONE: TokenFontStyle = TokenFontStyle {
        bold: false,
        italic: false,
        underline: false,
        strikethrough: false,
    };
    const BOLD: TokenFontStyle = TokenFontStyle { bold: true, ..NONE };
    const ITALIC: TokenFontStyle = TokenFontStyle {
        italic: true,
        ..NONE
    };

    fn color(hex: &str) -> Option<Hsla> {
        Some(parse_token_color(hex).unwrap())
    }

    fn font(style: &str) -> Option<TokenFontStyle> {
        Some(TokenFontStyle::parse(style))
    }

    fn rule(foreground: &str, font_style: TokenFontStyle) -> TrieRule {
        TrieRule {
            foreground: color(foreground),
            font_style,
        }
    }

    #[test]
    fn gives_higher_priority_to_deeper_matches() {
        let theme = TokenTrie::new([
            ("", color("100000"), None),
            (
                "punctuation.definition.string.begin.html",
                color("300000"),
                None,
            ),
            ("punctuation.definition.string", color("400000"), None),
        ]);
        let (actual, matched) = theme.match_token("punctuation.definition.string.begin.html");
        assert_eq!(actual, rule("300000", NONE));
        assert!(matched);
    }

    #[test]
    fn can_match() {
        let theme = TokenTrie::new([
            ("", color("F8F8F2"), None),
            ("source", None, None),
            ("something", None, None),
            ("bar", None, None),
            ("baz", None, None),
            ("bar", None, font("bold")),
            ("constant", color("300000"), font("italic")),
            ("constant.numeric", color("400000"), None),
            ("constant.numeric.hex", None, font("bold")),
            ("constant.numeric.oct", None, font("bold italic underline")),
            ("constant.numeric.bin", None, font("bold strikethrough")),
            ("constant.numeric.dec", color("500000"), font("")),
            ("storage.object.bar", color("600000"), font("")),
        ]);
        let check = |token: &str, expected: TrieRule| {
            assert_eq!(
                theme.match_token(token).0,
                expected,
                "when matching {token}"
            );
        };
        let a = "F8F8F2";
        // Defaults.
        for token in ["", "bazz", "asdfg"] {
            check(token, rule(a, NONE));
            assert!(!theme.match_token(token).1);
        }
        for token in [
            "source",
            "source.ts",
            "source.tss",
            "something",
            "something.ts",
        ] {
            check(token, rule(a, NONE));
        }
        for token in ["baz", "baz.ts", "baz.tss"] {
            check(token, rule(a, NONE));
        }
        for token in ["constant", "constant.string", "constant.hex"] {
            check(token, rule("300000", ITALIC));
        }
        check("constant.numeric", rule("400000", ITALIC));
        check("constant.numeric.baz", rule("400000", ITALIC));
        check("constant.numeric.hex", rule("400000", BOLD));
        check("constant.numeric.hex.baz", rule("400000", BOLD));
        let oct = TokenFontStyle {
            bold: true,
            italic: true,
            underline: true,
            strikethrough: false,
        };
        check("constant.numeric.oct", rule("400000", oct));
        check("constant.numeric.oct.baz", rule("400000", oct));
        let bin = TokenFontStyle {
            strikethrough: true,
            ..BOLD
        };
        check("constant.numeric.bin", rule("400000", bin));
        check("constant.numeric.bin.baz", rule("400000", bin));
        check("constant.numeric.dec", rule("500000", NONE));
        check("constant.numeric.dec.baz", rule("500000", NONE));
        check("storage.object.bar", rule("600000", NONE));
        check("storage.object.bar.baz", rule("600000", NONE));
        // Not `storage.object.bar`.
        check("storage.object.bart", rule(a, NONE));
        check("storage.object", rule(a, NONE));
        check("storage", rule(a, NONE));
        check("bar", rule(a, BOLD));
    }

    #[test]
    fn can_parse_font_styles() {
        assert_eq!(TokenFontStyle::parse(""), NONE);
        assert_eq!(TokenFontStyle::parse("bold"), BOLD);
        assert_eq!(
            TokenFontStyle::parse("bold italic underline"),
            TokenFontStyle {
                bold: true,
                italic: true,
                underline: true,
                strikethrough: false,
            }
        );
        // Whole words only, split on spaces, as Monaco's `parseTokenTheme`.
        assert_eq!(TokenFontStyle::parse("bolder"), NONE);
        assert_eq!(TokenFontStyle::parse("italic,bold"), NONE);
    }

    #[test]
    fn strcmp_works() {
        let mut actual = vec!["bar", "z", "zu", "a", "ab", ""];
        actual.sort();
        assert_eq!(actual, ["", "a", "ab", "bar", "z", "zu"]);
    }

    #[test]
    fn always_has_defaults() {
        let theme = TokenTrie::new([]);
        assert_eq!(theme.match_token("anything"), (TrieRule::default(), false));
        assert_eq!(theme.root(), None);
    }

    #[test]
    fn respects_incoming_defaults() {
        let root = |rules: Vec<(&'static str, Option<Hsla>, Option<TokenFontStyle>)>| {
            TokenTrie::new(rules).match_token("").0
        };
        assert_eq!(root(vec![("", None, None)]), TrieRule::default());
        assert_eq!(root(vec![("", None, Some(NONE))]), TrieRule::default());
        assert_eq!(
            root(vec![("", None, Some(BOLD))]),
            TrieRule {
                foreground: None,
                font_style: BOLD
            }
        );
        assert_eq!(
            root(vec![("", color("ff0000"), None)]),
            rule("ff0000", NONE)
        );
    }

    #[test]
    fn can_merge_incoming_defaults() {
        let theme = TokenTrie::new([
            ("", None, None),
            ("", color("00ff00"), None),
            ("", None, Some(BOLD)),
        ]);
        assert_eq!(theme.match_token("").0, rule("00ff00", BOLD));
        assert_eq!(theme.root(), Some(rule("00ff00", BOLD).style()));
    }

    #[test]
    fn defaults_are_inherited() {
        let theme = TokenTrie::new([("", color("F8F8F2"), None), ("var", color("ff0000"), None)]);
        assert_eq!(theme.match_token("var").0, rule("ff0000", NONE));
        assert_eq!(theme.match_token("other").0, rule("F8F8F2", NONE));
    }

    #[test]
    fn same_rules_get_merged() {
        let theme = TokenTrie::new([
            ("", color("F8F8F2"), None),
            ("var", None, Some(BOLD)),
            ("var", color("ff0000"), None),
        ]);
        assert_eq!(theme.match_token("var").0, rule("ff0000", BOLD));
    }

    #[test]
    fn rules_are_inherited() {
        let theme = TokenTrie::new([
            ("", color("F8F8F2"), None),
            ("var", color("ff0000"), Some(BOLD)),
            ("var.identifier", color("00ff00"), None),
            ("constant", color("100000"), Some(ITALIC)),
            ("constant.numeric", color("200000"), None),
            ("constant.numeric.hex", None, Some(BOLD)),
            ("constant.numeric.oct", None, font("bold italic underline")),
            ("constant.numeric.dec", color("300000"), Some(NONE)),
        ]);
        let check = |token: &str, expected: TrieRule| {
            assert_eq!(
                theme.match_token(token).0,
                expected,
                "when matching {token}"
            );
        };
        check("var", rule("ff0000", BOLD));
        check("var.identifier", rule("00ff00", BOLD));
        check("constant", rule("100000", ITALIC));
        check("constant.numeric", rule("200000", ITALIC));
        check("constant.numeric.hex", rule("200000", BOLD));
        check(
            "constant.numeric.oct",
            rule("200000", TokenFontStyle::parse("bold italic underline")),
        );
        check("constant.numeric.dec", rule("300000", NONE));
    }

    #[test]
    fn parent_rules_apply_whatever_their_order() {
        // Sorting puts `comment` before `comment.doc` either way.
        let theme = TokenTrie::new([
            ("comment.doc", None, Some(ITALIC)),
            ("comment", color("608B4E"), None),
        ]);
        assert_eq!(
            theme.match_token("comment.doc.rust").0,
            rule("608B4E", ITALIC)
        );
    }
}
