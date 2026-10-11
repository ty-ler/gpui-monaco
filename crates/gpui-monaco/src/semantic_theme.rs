//! How a theme styles semantic tokens.
//!
//! Follows VS Code's `TokenClassificationRegistry` (MIT): a theme's
//! `semanticTokenColors` rules are matched first, scored by selector (a
//! language match counts 10, a type match 1, each required modifier 100),
//! each style property taken from the best rule that sets it. Otherwise
//! the token type and modifiers map to TextMate scopes (the registry's
//! default `scopesToProbe`) matched against the theme's `tokenColors`.
//! Last, Monaco's standalone themes match `type.modifier…` against their
//! rules (`StandaloneThemeService.getTokenStyleMetadata`).

use gpui::Hsla;

use crate::theme::{Theme, TokenFontStyle, TokenStyle, parse_hex_color};

/// A semantic token's style: only the properties it sets override the
/// syntax highlighting's (Monaco's `SEMANTIC_USE_*` flags).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SemanticStyle {
    /// The color, if set.
    pub foreground: Option<Hsla>,
    /// Bold on or off, if set.
    pub bold: Option<bool>,
    /// Italic on or off, if set.
    pub italic: Option<bool>,
    /// Underline on or off, if set.
    pub underline: Option<bool>,
    /// Strikethrough on or off, if set.
    pub strikethrough: Option<bool>,
}

impl SemanticStyle {
    /// `base` with this style's properties applied.
    pub fn apply(&self, base: TokenStyle) -> TokenStyle {
        TokenStyle {
            foreground: self.foreground.or(base.foreground),
            bold: self.bold.unwrap_or(base.bold),
            italic: self.italic.unwrap_or(base.italic),
            underline: self.underline.unwrap_or(base.underline),
            strikethrough: self.strikethrough.unwrap_or(base.strikethrough),
            opacity: base.opacity,
        }
    }

    /// A theme rule's style, where only `true` font styles count as set.
    fn from_rule(style: TokenStyle) -> Self {
        let set = |on: bool| on.then_some(true);
        Self {
            foreground: style.foreground,
            bold: set(style.bold),
            italic: set(style.italic),
            underline: set(style.underline),
            strikethrough: set(style.strikethrough),
        }
    }
}

/// One `semanticTokenColors` entry, e.g. `"variable.readonly:rust"`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SemanticTokenRule {
    /// The token type, or `*` for any.
    token_type: String,
    modifiers: Vec<String>,
    language: Option<String>,
    foreground: Option<Hsla>,
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    strikethrough: Option<bool>,
}

impl SemanticTokenRule {
    /// Parses a selector and its value: a color string, or an object with
    /// `foreground`, `fontStyle`, `bold`, `italic`, `underline` and
    /// `strikethrough`.
    pub(crate) fn parse(selector: &str, value: &serde_json::Value) -> Option<Self> {
        let (selector, language) = match selector.split_once(':') {
            Some((selector, language)) => (selector, Some(language.trim().to_string())),
            None => (selector, None),
        };
        let mut parts = selector.trim().split('.');
        let token_type = parts.next()?.to_string();
        let modifiers = parts.map(str::to_string).collect();
        let mut rule = Self {
            token_type,
            modifiers,
            language,
            foreground: None,
            bold: None,
            italic: None,
            underline: None,
            strikethrough: None,
        };
        match value {
            serde_json::Value::String(color) => rule.foreground = parse_hex_color(color),
            serde_json::Value::Object(style) => {
                rule.foreground = style
                    .get("foreground")
                    .and_then(|c| c.as_str())
                    .and_then(parse_hex_color);
                if let Some(font_style) = style.get("fontStyle").and_then(|f| f.as_str()) {
                    let font_style = TokenFontStyle::parse(font_style);
                    rule.bold = Some(font_style.bold);
                    rule.italic = Some(font_style.italic);
                    rule.underline = Some(font_style.underline);
                    rule.strikethrough = Some(font_style.strikethrough);
                }
                let flag = |name: &str| style.get(name).and_then(|v| v.as_bool());
                rule.bold = flag("bold").or(rule.bold);
                rule.italic = flag("italic").or(rule.italic);
                rule.underline = flag("underline").or(rule.underline);
                rule.strikethrough = flag("strikethrough").or(rule.strikethrough);
            }
            _ => return None,
        }
        Some(rule)
    }

    /// VS Code `TokenSelector.match`: -1 for no match, else higher for a
    /// more specific selector.
    fn score(&self, token_type: &str, modifiers: &[&str], language: Option<&str>) -> i32 {
        let mut score = 0;
        if let Some(selector_language) = &self.language {
            if Some(selector_language.as_str()) != language {
                return -1;
            }
            score += 10;
        }
        if self.token_type != "*" {
            if self.token_type != token_type {
                return -1;
            }
            score += 1;
        }
        if self
            .modifiers
            .iter()
            .any(|m| !modifiers.contains(&m.as_str()))
        {
            return -1;
        }
        score + self.modifiers.len() as i32 * 100
    }
}

/// VS Code's default TextMate scopes for semantic token types and
/// `type.modifier` combinations (`registerTokenType` /
/// `registerTokenStyleDefault` in `tokenClassificationRegistry.ts`); the
/// last segment of each probed scope.
const SCOPES: &[(&str, &[&str])] = &[
    ("comment", &["comment"]),
    ("string", &["string"]),
    ("keyword", &["keyword.control"]),
    ("number", &["constant.numeric"]),
    ("regexp", &["constant.regexp"]),
    ("operator", &["keyword.operator"]),
    ("namespace", &["entity.name.namespace"]),
    ("type", &["entity.name.type", "support.type"]),
    ("struct", &["entity.name.type.struct"]),
    ("class", &["entity.name.type.class", "support.class"]),
    ("interface", &["entity.name.type.interface"]),
    ("enum", &["entity.name.type.enum"]),
    ("typeParameter", &["entity.name.type.parameter"]),
    ("function", &["entity.name.function", "support.function"]),
    (
        "member",
        &["entity.name.function.member", "support.function"],
    ),
    (
        "method",
        &["entity.name.function.member", "support.function"],
    ),
    ("macro", &["entity.name.function.preprocessor"]),
    (
        "variable",
        &["variable.other.readwrite", "entity.name.variable"],
    ),
    ("parameter", &["variable.parameter"]),
    ("property", &["variable.other.property"]),
    ("enumMember", &["variable.other.enummember"]),
    ("event", &["variable.other.event"]),
    (
        "decorator",
        &["entity.name.decorator", "entity.name.function"],
    ),
    ("variable.readonly", &["variable.other.constant"]),
    ("property.readonly", &["variable.other.constant.property"]),
    ("type.defaultLibrary", &["support.type"]),
    ("class.defaultLibrary", &["support.class"]),
    ("interface.defaultLibrary", &["support.class"]),
    (
        "variable.defaultLibrary",
        &["support.variable", "support.other.variable"],
    ),
    ("variable.readonly.defaultLibrary", &["support.constant"]),
    (
        "property.readonly.defaultLibrary",
        &["support.constant.property"],
    ),
    ("property.defaultLibrary", &["support.variable.property"]),
    ("function.defaultLibrary", &["support.function"]),
    ("member.defaultLibrary", &["support.function"]),
];

impl Theme {
    /// The style for a semantic token, or `None` to keep the syntax
    /// highlighting's.
    pub fn semantic_style(
        &self,
        token_type: &str,
        modifiers: &[&str],
        language: Option<&str>,
    ) -> Option<SemanticStyle> {
        let key = format!(
            "{}:{token_type}.{}",
            language.unwrap_or_default(),
            modifiers.join(".")
        );
        let mut cache = self
            .semantic_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *cache
            .entry(key)
            .or_insert_with(|| self.resolve_semantic(token_type, modifiers, language))
    }

    fn resolve_semantic(
        &self,
        token_type: &str,
        modifiers: &[&str],
        language: Option<&str>,
    ) -> Option<SemanticStyle> {
        // `semanticTokenColors`: each property from the best rule setting it.
        let scored: Vec<(i32, &SemanticTokenRule)> = self
            .semantic_rules
            .iter()
            .map(|rule| (rule.score(token_type, modifiers, language), rule))
            .filter(|(score, _)| *score >= 0)
            .collect();
        if !scored.is_empty() {
            fn best<T: Copy>(
                scored: &[(i32, &SemanticTokenRule)],
                get: impl Fn(&SemanticTokenRule) -> Option<T>,
            ) -> Option<T> {
                // Later rules win ties, as in VS Code.
                scored
                    .iter()
                    .filter_map(|(score, rule)| get(rule).map(|v| (*score, v)))
                    .fold(None, |acc: Option<(i32, T)>, (score, value)| match acc {
                        Some((best, _)) if best > score => acc,
                        _ => Some((score, value)),
                    })
                    .map(|(_, v)| v)
            }
            let foreground = best(&scored, |r| r.foreground);
            let bold = best(&scored, |r| r.bold);
            let italic = best(&scored, |r| r.italic);
            let underline = best(&scored, |r| r.underline);
            let strikethrough = best(&scored, |r| r.strikethrough);
            if foreground.is_some()
                || bold.is_some()
                || italic.is_some()
                || underline.is_some()
                || strikethrough.is_some()
            {
                return Some(SemanticStyle {
                    foreground,
                    bold,
                    italic,
                    underline,
                    strikethrough,
                });
            }
        }
        // The default TextMate scopes, most specific combination first.
        let mut candidates: Vec<(usize, &[&str])> = SCOPES
            .iter()
            .filter_map(|(selector, scopes)| {
                let mut parts = selector.split('.');
                let ty = parts.next()?;
                let required: Vec<&str> = parts.collect();
                (ty == token_type && required.iter().all(|m| modifiers.contains(m)))
                    .then_some((required.len(), *scopes))
            })
            .collect();
        candidates.sort_by_key(|(specificity, _)| std::cmp::Reverse(*specificity));
        for (_, scopes) in candidates {
            for scope in scopes {
                if let Some(style) = self.textmate_style(scope) {
                    return Some(SemanticStyle::from_rule(style));
                }
            }
        }
        // Monaco's standalone themes: `type.modifier…` against their rules.
        let name = std::iter::once(token_type)
            .chain(modifiers.iter().copied())
            .collect::<Vec<_>>()
            .join(".");
        // Monaco's standalone theme always has a style, the default rule's
        // when nothing else matches, and sets every property: such tokens
        // fully replace the syntax style.
        self.monaco_style(&name).map(|style| SemanticStyle {
            foreground: Some(style.foreground.unwrap_or(self.colors.foreground)),
            bold: Some(style.bold),
            italic: Some(style.italic),
            underline: Some(style.underline),
            strikethrough: Some(style.strikethrough),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::rgb;

    fn theme(json: &str) -> Theme {
        Theme::from_vscode_json_with_includes(json, &|_| None).unwrap()
    }

    fn color(style: Option<SemanticStyle>) -> Option<Hsla> {
        style.and_then(|s| s.foreground)
    }

    #[test]
    fn semantic_token_colors_by_score() {
        let theme = theme(
            r##"{
                "semanticHighlighting": true,
                "semanticTokenColors": {
                    "variable": "#111111",
                    "variable.readonly": { "foreground": "#222222", "bold": true },
                    "*.readonly": "#333333",
                    "variable:rust": "#444444",
                    "function": { "fontStyle": "italic" }
                }
            }"##,
        );
        assert!(theme.semantic_highlighting);
        let red = |hex| Some(Hsla::from(rgb(hex)));
        assert_eq!(
            color(theme.semantic_style("variable", &[], None)),
            red(0x111111)
        );
        // A language match outscores the plain type.
        assert_eq!(
            color(theme.semantic_style("variable", &[], Some("rust"))),
            red(0x444444)
        );
        // Modifiers outscore both; the type match breaks the tie with `*`.
        let readonly = theme.semantic_style("variable", &["readonly"], Some("rust"));
        assert_eq!(color(readonly), red(0x222222));
        assert_eq!(readonly.unwrap().bold, Some(true));
        assert_eq!(
            color(theme.semantic_style("property", &["readonly"], None)),
            red(0x333333)
        );
        // Only a font style: no color of its own.
        let function = theme.semantic_style("function", &[], None).unwrap();
        assert!(function.italic == Some(true) && function.foreground.is_none());
        // Unknown: no style.
        assert_eq!(theme.semantic_style("label", &[], None), None);
    }

    #[test]
    fn falls_back_to_textmate_scopes() {
        let theme = theme(
            r##"{
                "tokenColors": [
                    { "scope": "variable.other.constant", "settings": { "foreground": "#aa0000" } },
                    { "scope": "entity.name.function", "settings": { "foreground": "#00aa00" } },
                    { "scope": "support.function", "settings": { "foreground": "#0000aa" } }
                ]
            }"##,
        );
        assert!(!theme.semantic_highlighting);
        let hex = |hex| Some(Hsla::from(rgb(hex)));
        assert_eq!(
            color(theme.semantic_style("variable", &["readonly"], None)),
            hex(0xaa0000)
        );
        assert_eq!(
            color(theme.semantic_style("function", &[], None)),
            hex(0x00aa00)
        );
        // `function.defaultLibrary` probes `support.function` first.
        assert_eq!(
            color(theme.semantic_style("function", &["defaultLibrary"], None)),
            hex(0x0000aa)
        );
    }

    #[test]
    fn standalone_themes_match_type_and_modifiers() {
        let theme = Theme::new(
            "standalone",
            crate::theme::ThemeKind::Dark,
            crate::theme::EditorColors::vs_dark(),
            vec![
                crate::theme::TokenRule::monaco("variable", Some("123456"), None).unwrap(),
                crate::theme::TokenRule::monaco("variable.readonly", Some("654321"), None).unwrap(),
            ],
        );
        let hex = |hex| Some(Hsla::from(rgb(hex)));
        assert_eq!(
            color(theme.semantic_style("variable", &[], None)),
            hex(0x123456)
        );
        assert_eq!(
            color(theme.semantic_style("variable", &["readonly", "static"], None)),
            hex(0x654321)
        );
        // Unmatched: the default style (the editor foreground, no font
        // style), replacing the syntax style as in Monaco.
        let unmatched = theme.semantic_style("label", &[], None).unwrap();
        assert_eq!(unmatched.foreground, Some(theme.colors.foreground));
        assert_eq!(unmatched.bold, Some(false));
    }
}
