//! Themes: editor chrome colors plus token colors.
//!
//! Two kinds of token rules are supported and can be mixed:
//! - **Monaco rules** keyed by Monarch token names (`keyword`, `string.key.json`),
//!   as in Monaco's built-in themes and `monaco.editor.defineTheme`. The
//!   built-in [`Theme::vs`] and [`Theme::vs_dark`] are ported from Monaco's
//!   `standalone/common/themes.ts` (MIT).
//! - **TextMate rules** from VS Code theme JSON `tokenColors`, keyed by scope
//!   selectors (`entity.name.function`, `keyword.control`).
//!
//! Tree-sitter capture names (e.g. `function.method`) are mapped onto both
//! vocabularies by [`monaco_token_for_capture`] and [`textmate_scopes_for_capture`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use gpui::{Hsla, Rgba, rgb, rgba};
use serde::Deserialize;
use syntax::HighlightId;

#[derive(Clone, Debug, PartialEq)]
pub struct EditorColors {
    pub background: Hsla,
    pub foreground: Hsla,
    pub line_number: Hsla,
    pub active_line_number: Hsla,
    pub line_highlight_background: Option<Hsla>,
    pub line_highlight_border: Option<Hsla>,
    pub selection_background: Hsla,
    pub inactive_selection_background: Hsla,
    pub cursor: Hsla,
    pub gutter_background: Hsla,
    pub scrollbar_slider: Hsla,
    pub scrollbar_slider_hover: Hsla,
    pub scrollbar_slider_active: Hsla,
    pub scrollbar_shadow: Hsla,
    pub ime_underline: Hsla,
    /// Current find match (`editor.findMatchBackground`).
    pub find_match_background: Hsla,
    /// Other find matches (`editor.findMatchHighlightBackground`).
    pub find_match_highlight_background: Hsla,
    /// Find matches in the overview ruler (`editorOverviewRuler.findMatchForeground`).
    pub find_match_overview: Hsla,
    /// Widget chrome (`editorWidget.background` / `.foreground` / `.border`).
    pub widget_background: Hsla,
    pub widget_foreground: Hsla,
    pub widget_border: Hsla,
    /// Input boxes in widgets (`input.background` / `inputOption.activeBackground`).
    pub input_background: Hsla,
    pub input_option_active_background: Hsla,
    pub input_option_active_border: Hsla,
    /// `editorBracketMatch.background` / `.border`.
    pub bracket_match_background: Hsla,
    pub bracket_match_border: Hsla,
}

fn c(color: Rgba) -> Hsla {
    color.into()
}

impl EditorColors {
    /// Monaco `vs-dark`.
    pub fn vs_dark() -> Self {
        Self {
            background: c(rgb(0x1e1e1e)),
            foreground: c(rgb(0xd4d4d4)),
            line_number: c(rgb(0x858585)),
            active_line_number: c(rgb(0xc6c6c6)),
            line_highlight_background: None,
            line_highlight_border: Some(c(rgb(0x282828))),
            selection_background: c(rgb(0x264f78)),
            inactive_selection_background: c(rgb(0x3a3d41)),
            cursor: c(rgb(0xaeafad)),
            gutter_background: c(rgb(0x1e1e1e)),
            scrollbar_slider: c(rgba(0x79797966)),
            scrollbar_slider_hover: c(rgba(0x646464b3)),
            scrollbar_slider_active: c(rgba(0xbfbfbf66)),
            scrollbar_shadow: c(rgb(0x000000)),
            ime_underline: c(rgb(0xd4d4d4)),
            find_match_background: c(rgb(0x515c6a)),
            find_match_highlight_background: c(rgba(0xea5c0055)),
            find_match_overview: c(rgba(0xd186167e)),
            widget_background: c(rgb(0x252526)),
            widget_foreground: c(rgb(0xcccccc)),
            widget_border: c(rgb(0x454545)),
            input_background: c(rgb(0x3c3c3c)),
            input_option_active_background: c(rgba(0x007fd466)),
            input_option_active_border: c(rgb(0x007acc)),
            bracket_match_background: c(rgba(0x0064001a)),
            bracket_match_border: c(rgb(0x888888)),
        }
    }

    /// Monaco `vs`.
    pub fn vs() -> Self {
        Self {
            background: c(rgb(0xfffffe)),
            foreground: c(rgb(0x000000)),
            line_number: c(rgb(0x237893)),
            active_line_number: c(rgb(0x0b216f)),
            line_highlight_background: None,
            line_highlight_border: Some(c(rgb(0xeeeeee))),
            selection_background: c(rgb(0xadd6ff)),
            inactive_selection_background: c(rgb(0xe5ebf1)),
            cursor: c(rgb(0x000000)),
            gutter_background: c(rgb(0xfffffe)),
            scrollbar_slider: c(rgba(0x64646466)),
            scrollbar_slider_hover: c(rgba(0x646464b3)),
            scrollbar_slider_active: c(rgba(0x00000099)),
            scrollbar_shadow: c(rgb(0xdddddd)),
            ime_underline: c(rgb(0x000000)),
            find_match_background: c(rgb(0xa8ac94)),
            find_match_highlight_background: c(rgba(0xea5c0055)),
            find_match_overview: c(rgba(0xd186167e)),
            widget_background: c(rgb(0xf3f3f3)),
            widget_foreground: c(rgb(0x616161)),
            widget_border: c(rgb(0xc8c8c8)),
            input_background: c(rgb(0xffffff)),
            input_option_active_background: c(rgba(0x007fd433)),
            input_option_active_border: c(rgb(0x007acc)),
            bracket_match_background: c(rgba(0x0064001a)),
            bracket_match_border: c(rgb(0xb9b9b9)),
        }
    }

    /// Applies VS Code workbench color keys (`editor.background`, ...).
    fn apply_vscode_colors(&mut self, colors: &HashMap<String, String>) {
        let get = |key: &str| colors.get(key).and_then(|value| parse_hex_color(value));
        let set = |slot: &mut Hsla, key: &str| {
            if let Some(color) = get(key) {
                *slot = color;
            }
        };
        set(&mut self.background, "editor.background");
        set(&mut self.foreground, "editor.foreground");
        set(&mut self.line_number, "editorLineNumber.foreground");
        set(
            &mut self.active_line_number,
            "editorLineNumber.activeForeground",
        );
        set(&mut self.selection_background, "editor.selectionBackground");
        set(
            &mut self.inactive_selection_background,
            "editor.inactiveSelectionBackground",
        );
        set(&mut self.cursor, "editorCursor.foreground");
        set(&mut self.scrollbar_slider, "scrollbarSlider.background");
        set(
            &mut self.scrollbar_slider_hover,
            "scrollbarSlider.hoverBackground",
        );
        set(
            &mut self.scrollbar_slider_active,
            "scrollbarSlider.activeBackground",
        );
        set(&mut self.scrollbar_shadow, "scrollbar.shadow");
        set(
            &mut self.find_match_background,
            "editor.findMatchBackground",
        );
        set(
            &mut self.find_match_highlight_background,
            "editor.findMatchHighlightBackground",
        );
        set(
            &mut self.find_match_overview,
            "editorOverviewRuler.findMatchForeground",
        );
        set(&mut self.widget_background, "editorWidget.background");
        set(&mut self.widget_foreground, "editorWidget.foreground");
        set(&mut self.widget_border, "editorWidget.border");
        set(&mut self.input_background, "input.background");
        set(
            &mut self.input_option_active_background,
            "inputOption.activeBackground",
        );
        set(
            &mut self.input_option_active_border,
            "inputOption.activeBorder",
        );
        self.gutter_background = get("editorGutter.background").unwrap_or(self.background);
        if let Some(color) = get("editor.lineHighlightBackground") {
            self.line_highlight_background = Some(color);
        }
        if let Some(color) = get("editor.lineHighlightBorder") {
            self.line_highlight_border = Some(color);
        } else if self.line_highlight_background.is_some() {
            self.line_highlight_border = None;
        }
        self.ime_underline = self.foreground;
    }
}

impl Default for EditorColors {
    fn default() -> Self {
        Self::vs_dark()
    }
}

/// `#RGB`, `#RGBA`, `#RRGGBB` or `#RRGGBBAA`.
pub fn parse_hex_color(value: &str) -> Option<Hsla> {
    let hex = value.trim().strip_prefix('#')?;
    let expand = |s: &str| -> String { s.chars().flat_map(|c| [c, c]).collect() };
    let full = match hex.len() {
        3 => format!("{}ff", expand(hex)),
        4 => expand(hex),
        6 => format!("{hex}ff"),
        8 => hex.to_string(),
        _ => return None,
    };
    let value = u32::from_str_radix(&full, 16).ok()?;
    Some(rgba(value).into())
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TokenStyle {
    pub foreground: Option<Hsla>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
}

impl TokenStyle {
    fn from_font_style(foreground: Option<Hsla>, font_style: Option<&str>) -> Self {
        let font_style = font_style.unwrap_or_default();
        Self {
            foreground,
            bold: font_style.contains("bold"),
            italic: font_style.contains("italic"),
            underline: font_style.contains("underline"),
            strikethrough: font_style.contains("strikethrough"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleKind {
    /// Selector is a Monarch token name.
    Monaco,
    /// Selector is a TextMate scope.
    TextMate,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TokenRule {
    pub kind: RuleKind,
    pub selector: String,
    pub style: TokenStyle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeKind {
    Light,
    Dark,
}

/// Editor colors and token rules.
pub struct Theme {
    pub name: String,
    pub kind: ThemeKind,
    pub colors: EditorColors,
    rules: Vec<TokenRule>,
    cache: Mutex<HashMap<HighlightId, TokenStyle>>,
}

impl Clone for Theme {
    fn clone(&self) -> Self {
        Self::new(
            self.name.clone(),
            self.kind,
            self.colors.clone(),
            self.rules.clone(),
        )
    }
}

impl std::fmt::Debug for Theme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Theme").field("name", &self.name).finish()
    }
}

macro_rules! monaco_rules {
    ($($token:literal => $color:literal $(, $style:literal)?);* $(;)?) => {
        vec![$(TokenRule {
            kind: RuleKind::Monaco,
            selector: $token.to_string(),
            style: TokenStyle::from_font_style(
                if $color.is_empty() { None } else { parse_hex_color(concat!("#", $color)) },
                None $(.or(Some($style)))?,
            ),
        }),*]
    };
}

impl Theme {
    pub fn new(
        name: impl Into<String>,
        kind: ThemeKind,
        colors: EditorColors,
        rules: Vec<TokenRule>,
    ) -> Self {
        Self {
            name: name.into(),
            kind,
            colors,
            rules,
            cache: Mutex::default(),
        }
    }

    pub fn rules(&self) -> &[TokenRule] {
        &self.rules
    }

    /// Monaco's `vs-dark`.
    pub fn vs_dark() -> Self {
        let rules = monaco_rules! {
            "invalid" => "f44747";
            "emphasis" => "", "italic";
            "strong" => "", "bold";
            "variable" => "74B0DF";
            "variable.predefined" => "4864AA";
            "variable.parameter" => "9CDCFE";
            "constant" => "569CD6";
            "comment" => "608B4E";
            "number" => "B5CEA8";
            "number.hex" => "5BB498";
            "regexp" => "B46695";
            "annotation" => "cc6666";
            "type" => "3DC9B0";
            "delimiter" => "DCDCDC";
            "delimiter.html" => "808080";
            "delimiter.xml" => "808080";
            "tag" => "569CD6";
            "meta.tag" => "CE9178";
            "metatag" => "DD6A6F";
            "key" => "9CDCFE";
            "string.key.json" => "9CDCFE";
            "string.value.json" => "CE9178";
            "attribute.name" => "9CDCFE";
            "attribute.value" => "CE9178";
            "string" => "CE9178";
            "keyword" => "569CD6";
            "keyword.flow" => "C586C0";
            "keyword.json" => "CE9178";
        };
        Self::new("vs-dark", ThemeKind::Dark, EditorColors::vs_dark(), rules)
    }

    /// Monaco's `vs`.
    pub fn vs() -> Self {
        let rules = monaco_rules! {
            "invalid" => "cd3131";
            "emphasis" => "", "italic";
            "strong" => "", "bold";
            "variable" => "001188";
            "variable.predefined" => "4864AA";
            "constant" => "dd0000";
            "comment" => "008000";
            "number" => "098658";
            "number.hex" => "3030c0";
            "regexp" => "800000";
            "annotation" => "808080";
            "type" => "008080";
            "delimiter" => "000000";
            "delimiter.html" => "383838";
            "delimiter.xml" => "0000FF";
            "tag" => "800000";
            "metatag" => "e00000";
            "key" => "863B00";
            "string.key.json" => "A31515";
            "string.value.json" => "0451A5";
            "attribute.name" => "FF0000";
            "attribute.value" => "0451A5";
            "string" => "A31515";
            "keyword" => "0000FF";
            "keyword.json" => "0451A5";
            "keyword.flow" => "AF00DB";
        };
        Self::new("vs", ThemeKind::Light, EditorColors::vs(), rules)
    }

    /// Monaco `defineTheme`: `base` plus extra Monaco rules and VS Code color
    /// keys. With `inherit: false` the base's token rules are dropped.
    pub fn define(
        name: impl Into<String>,
        base: &Theme,
        inherit: bool,
        rules: Vec<TokenRule>,
        colors: &HashMap<String, String>,
    ) -> Self {
        let mut all_rules = if inherit {
            base.rules.clone()
        } else {
            Vec::new()
        };
        all_rules.extend(rules);
        let mut editor_colors = base.colors.clone();
        editor_colors.apply_vscode_colors(colors);
        Self::new(name, base.kind, editor_colors, all_rules)
    }

    /// Loads a VS Code color theme (`colors` + `tokenColors`). An `include`
    /// is ignored; use [`Theme::from_vscode_json_with_includes`] or
    /// [`Theme::from_vscode_file`] to follow it.
    pub fn from_vscode_json(json: &str) -> Result<Self, serde_json::Error> {
        Self::from_vscode_json_with_includes(json, &|_| None)
    }

    /// Like [`Theme::from_vscode_json`], resolving `include` paths (as written
    /// in the JSON, e.g. `./dark_vs.json`) with `resolve`. Included themes are
    /// applied first, so the including theme's colors and rules win.
    pub fn from_vscode_json_with_includes(
        json: &str,
        resolve: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Self, serde_json::Error> {
        let mut merged = VsCodeTheme::default();
        load_vscode_theme(json, resolve, 0, &mut merged)?;
        let kind = match merged.kind.as_deref() {
            Some("light") | Some("hcLight") => ThemeKind::Light,
            _ => ThemeKind::Dark,
        };
        let mut colors = match kind {
            ThemeKind::Light => EditorColors::vs(),
            ThemeKind::Dark => EditorColors::vs_dark(),
        };
        colors.apply_vscode_colors(&merged.colors);
        let mut rules = Vec::new();
        for token_color in merged.token_colors {
            let style = TokenStyle::from_font_style(
                token_color
                    .settings
                    .foreground
                    .as_deref()
                    .and_then(parse_hex_color),
                token_color.settings.font_style.as_deref(),
            );
            let scopes = match token_color.scope {
                None => continue,
                Some(Scope::One(scopes)) => scopes
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect::<Vec<_>>(),
                Some(Scope::Many(scopes)) => scopes,
            };
            for scope in scopes {
                // Descendant selectors ("meta.embedded string") are matched on
                // their last segment only.
                let Some(selector) = scope.split_whitespace().last() else {
                    continue;
                };
                rules.push(TokenRule {
                    kind: RuleKind::TextMate,
                    selector: selector.to_string(),
                    style,
                });
            }
        }
        Ok(Self::new(
            merged.name.unwrap_or_else(|| "VS Code theme".into()),
            kind,
            colors,
            rules,
        ))
    }

    /// Loads a VS Code theme file, following `include`s relative to it.
    pub fn from_vscode_file(path: &std::path::Path) -> std::io::Result<Self> {
        let json = std::fs::read_to_string(path)?;
        let dir = path
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_default();
        Self::from_vscode_json_with_includes(&json, &|include| {
            std::fs::read_to_string(dir.join(include)).ok()
        })
        .map_err(std::io::Error::other)
    }

    /// The style for a highlight capture.
    pub fn style_for(&self, highlight: HighlightId) -> TokenStyle {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        *cache
            .entry(highlight)
            .or_insert_with(|| self.resolve(&highlight.name()))
    }

    fn resolve(&self, capture: &str) -> TokenStyle {
        // TextMate rules: the first candidate scope with a matching rule wins;
        // among rules matching it, the longest (most specific) selector, and
        // later rules on ties.
        for scope in textmate_scopes_for_capture(capture) {
            if let Some(style) = best_match(&self.rules, RuleKind::TextMate, scope) {
                return style;
            }
        }
        if let Some(token) = monaco_token_for_capture(capture)
            && let Some(style) = best_match(&self.rules, RuleKind::Monaco, token)
        {
            return style;
        }
        TokenStyle::default()
    }
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VsCodeTheme {
    #[serde(default)]
    name: Option<String>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    include: Option<String>,
    #[serde(default)]
    colors: HashMap<String, String>,
    #[serde(default)]
    token_colors: Vec<TokenColor>,
}

#[derive(Deserialize)]
struct TokenColor {
    #[serde(default)]
    scope: Option<Scope>,
    settings: Settings,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Scope {
    One(String),
    Many(Vec<String>),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    #[serde(default)]
    foreground: Option<String>,
    #[serde(default)]
    font_style: Option<String>,
}

/// Merges `json` (and, first, whatever it includes) into `merged`.
fn load_vscode_theme(
    json: &str,
    resolve: &dyn Fn(&str) -> Option<String>,
    depth: usize,
    merged: &mut VsCodeTheme,
) -> Result<(), serde_json::Error> {
    let theme: VsCodeTheme = serde_json::from_str(&syntax::strip_jsonc(json))?;
    if depth < 8
        && let Some(include) = theme.include.as_deref()
        && let Some(included) = resolve(include)
    {
        load_vscode_theme(&included, resolve, depth + 1, merged)?;
    }
    if theme.name.is_some() {
        merged.name = theme.name;
    }
    if theme.kind.is_some() {
        merged.kind = theme.kind;
    }
    merged.colors.extend(theme.colors);
    merged.token_colors.extend(theme.token_colors);
    Ok(())
}

/// `selector` matches `name` if equal or a dot-segment prefix of it.
fn selector_matches(selector: &str, name: &str) -> bool {
    name == selector
        || (name.starts_with(selector) && name.as_bytes().get(selector.len()) == Some(&b'.'))
}

fn best_match(rules: &[TokenRule], kind: RuleKind, name: &str) -> Option<TokenStyle> {
    let mut best: Option<&TokenRule> = None;
    for rule in rules.iter().filter(|rule| rule.kind == kind) {
        if selector_matches(&rule.selector, name)
            && best.is_none_or(|b| rule.selector.len() >= b.selector.len())
        {
            best = Some(rule);
        }
    }
    best.map(|rule| rule.style)
}

/// The Monarch token a tree-sitter capture corresponds to in Monaco themes.
pub fn monaco_token_for_capture(capture: &str) -> Option<&'static str> {
    let first = capture.split('.').next().unwrap_or(capture);
    Some(match capture {
        "string.special.key" => "string.key.json",
        "string.regex" | "string.regexp" => "regexp",
        "variable.builtin" => "variable.predefined",
        "variable.parameter" => "variable.parameter",
        "constant.builtin" | "boolean" => "keyword",
        "number.hex" => "number.hex",
        "text.title" | "markup.heading" => "keyword",
        "text.emphasis" | "markup.italic" => "emphasis",
        "text.strong" | "markup.bold" => "strong",
        "text.literal" | "markup.raw" => "variable",
        "text.uri" | "text.reference" | "markup.link" => "string",
        _ => match first {
            "comment" => "comment",
            "string" | "character" => "string",
            "number" | "float" => "number",
            "keyword" | "conditional" | "repeat" | "include" | "exception" => "keyword",
            "type" | "constructor" => "type",
            "tag" => "tag",
            "attribute" => "attribute.name",
            "property" => "key",
            "constant" => "constant",
            "operator" | "punctuation" => "delimiter",
            "escape" => "string",
            "label" | "annotation" => "annotation",
            _ => return None,
        },
    })
}

/// Candidate TextMate scopes for a tree-sitter capture, most specific first.
pub fn textmate_scopes_for_capture(capture: &str) -> &'static [&'static str] {
    let first = capture.split('.').next().unwrap_or(capture);
    match capture {
        "comment.documentation" | "comment.doc" => &["comment.block.documentation", "comment"],
        "string.special.key" => &[
            "support.type.property-name.json",
            "support.type.property-name",
            "string",
        ],
        "string.escape" | "escape" => &["constant.character.escape"],
        "string.regex" | "string.regexp" => &["string.regexp"],
        "string.special" | "string.special.symbol" => &["constant.other.symbol", "string"],
        "constant.builtin" | "boolean" => &["constant.language"],
        "keyword.operator" | "operator" => &["keyword.operator"],
        "keyword.control"
        | "keyword.return"
        | "keyword.conditional"
        | "keyword.repeat"
        | "conditional"
        | "repeat"
        | "keyword.exception"
        | "exception" => &["keyword.control"],
        "keyword.function" | "keyword.storage" | "keyword.type" => &["storage.type", "keyword"],
        "function.macro" => &["entity.name.function.macro", "entity.name.function"],
        "function.builtin" => &["support.function", "entity.name.function"],
        "function.method" | "function.method.call" | "function.call" => &["entity.name.function"],
        "type.builtin" => &["support.type.primitive", "support.type", "entity.name.type"],
        "variable.builtin" => &["variable.language"],
        "variable.parameter" => &["variable.parameter"],
        "variable.member" | "property" => &[
            "variable.other.property",
            "support.type.property-name",
            "variable",
        ],
        "text.title" | "markup.heading" => &["markup.heading", "entity.name.section"],
        "text.emphasis" | "markup.italic" => &["markup.italic"],
        "text.strong" | "markup.bold" => &["markup.bold"],
        "text.literal" | "markup.raw" => &["markup.inline.raw", "markup.raw"],
        "text.uri" | "markup.link.url" => &["markup.underline.link"],
        "text.reference" | "markup.link" => &["string.other.link", "markup.underline.link"],
        _ => match first {
            "comment" => &["comment"],
            "string" | "character" => &["string"],
            "number" | "float" => &["constant.numeric"],
            "keyword" => &["keyword", "storage.type"],
            "function" => &["entity.name.function", "support.function"],
            "type" => &["entity.name.type", "support.type", "storage.type"],
            "constructor" => &[
                "entity.name.type.class",
                "entity.name.type",
                "entity.name.function",
            ],
            "variable" => &["variable.other.readwrite", "variable"],
            "constant" => &["variable.other.constant", "constant"],
            "attribute" => &["entity.other.attribute-name"],
            "tag" => &["entity.name.tag"],
            "label" => &["entity.name.label"],
            "namespace" | "module" => &["entity.name.namespace", "entity.name.type.module"],
            "punctuation" => &["punctuation"],
            "embedded" => &["meta.embedded"],
            _ => &[],
        },
    }
}

/// A shared theme handle.
pub type ThemeRef = Arc<Theme>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_colors() {
        assert_eq!(parse_hex_color("#fff"), Some(rgba(0xffffffff).into()));
        assert_eq!(parse_hex_color("#11223344"), Some(rgba(0x11223344).into()));
        assert_eq!(parse_hex_color("nope"), None);
    }

    #[test]
    fn monaco_rules_resolve_captures() {
        let theme = Theme::vs_dark();
        let keyword = theme.style_for(HighlightId::intern("keyword"));
        assert_eq!(keyword.foreground, parse_hex_color("#569CD6"));
        let key = theme.style_for(HighlightId::intern("string.special.key"));
        assert_eq!(key.foreground, parse_hex_color("#9CDCFE"));
        let function = theme.style_for(HighlightId::intern("function"));
        assert_eq!(
            function.foreground, None,
            "Monaco's vs-dark doesn't color functions"
        );
    }

    #[test]
    fn vscode_theme_json() {
        let json = r##"{
            // Dark+ excerpt
            "name": "Test Dark+",
            "type": "dark",
            "colors": { "editor.background": "#1E1E1E", "editorLineNumber.activeForeground": "#C6C6C6" },
            "tokenColors": [
                { "scope": "keyword", "settings": { "foreground": "#569CD6" } },
                { "scope": ["keyword.control", "keyword.control.flow"], "settings": { "foreground": "#C586C0" } },
                { "scope": "entity.name.function, support.function", "settings": { "foreground": "#DCDCAA" } },
                { "scope": "comment", "settings": { "foreground": "#6A9955", "fontStyle": "italic" } },
            ]
        }"##;
        let theme = Theme::from_vscode_json(json).unwrap();
        assert_eq!(theme.name, "Test Dark+");
        let function = theme.style_for(HighlightId::intern("function.method"));
        assert_eq!(function.foreground, parse_hex_color("#DCDCAA"));
        let flow = theme.style_for(HighlightId::intern("keyword.return"));
        assert_eq!(flow.foreground, parse_hex_color("#C586C0"));
        let comment = theme.style_for(HighlightId::intern("comment"));
        assert!(comment.italic);
    }
}
