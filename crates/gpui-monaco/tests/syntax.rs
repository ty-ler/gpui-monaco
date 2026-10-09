//! Background syntax parsing in `TextModel`.

use gpui::{AppContext as _, TestAppContext};
use gpui_monaco::TextModel;

#[gpui::test]
fn model_parses_in_background_and_after_edits(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry()));
    let model = cx.new(|_| TextModel::new("fn main() {}\n").with_language("rust"));
    model.update(cx, |model, cx| model.ensure_syntax(cx));
    cx.run_until_parked();
    let names = |model: &TextModel| {
        let syntax = model.syntax_snapshot().expect("parsed");
        syntax
            .highlighter
            .highlights(&syntax.state, syntax.text.rope(), 0..syntax.text.len())
            .iter()
            .map(|span| span.highlight.name().to_string())
            .collect::<Vec<_>>()
    };
    assert!(
        model
            .read_with(cx, |m, _| names(m))
            .contains(&"keyword".to_string())
    );

    model.update(cx, |model, cx| model.edit([(0..0, "// hi\n")], cx).unwrap());
    cx.run_until_parked();
    model.read_with(cx, |model, _| {
        let syntax = model.syntax_snapshot().unwrap();
        assert_eq!(syntax.text.version(), model.version());
        assert!(names(model).contains(&"comment".to_string()));
    });
}

#[test]
fn loads_vscode_dark_modern_with_includes() {
    use gpui_monaco::syntax::HighlightId;
    use gpui_monaco::theme::parse_hex_color;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/themes/dark_modern.json");
    let theme = gpui_monaco::Theme::from_vscode_file(&path).unwrap();
    assert_eq!(theme.name, "Dark Modern");
    // From dark_modern.json itself.
    assert_eq!(theme.colors.background, parse_hex_color("#1F1F1F").unwrap());
    // From dark_plus.json (included).
    assert_eq!(
        theme.style_for(HighlightId::intern("function")).foreground,
        parse_hex_color("#DCDCAA")
    );
    assert_eq!(
        theme
            .style_for(HighlightId::intern("keyword.return"))
            .foreground,
        parse_hex_color("#C586C0")
    );
    // From dark_vs.json (included by dark_plus).
    assert_eq!(
        theme.style_for(HighlightId::intern("comment")).foreground,
        parse_hex_color("#6A9955")
    );
    assert_eq!(
        theme.style_for(HighlightId::intern("string")).foreground,
        parse_hex_color("#CE9178")
    );
}

#[test]
fn vscode_theme_without_type_infers_light_from_background() {
    use gpui_monaco::theme::{ThemeKind, parse_hex_color};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/themes/light_plus.json");
    let theme = gpui_monaco::Theme::from_vscode_file(&path).unwrap();
    assert_eq!(theme.kind, ThemeKind::Light);
    // Unset colors fall back to Monaco's `vs` defaults, not `vs-dark`.
    assert_eq!(
        theme.colors.widget_background,
        parse_hex_color("#F3F3F3").unwrap()
    );
}

#[gpui::test]
fn language_with_custom_highlighter(cx: &mut TestAppContext) {
    use std::ops::Range;
    use std::sync::Arc;

    use gpui_monaco::syntax::{
        HighlightId, HighlightSpan, HighlightState, Highlighter, Language, LanguageRegistry,
    };
    use gpui_monaco::text::{BufferSnapshot, Patch, Rope};

    /// Highlights every `!` as a keyword; its state is the text it saw.
    struct Bangs;
    impl Highlighter for Bangs {
        fn update(
            &self,
            _previous: Option<(&HighlightState, &Rope, &Patch)>,
            text: &BufferSnapshot,
        ) -> HighlightState {
            HighlightState::new(text.rope().to_string())
        }
        fn highlights(
            &self,
            state: &HighlightState,
            _text: &Rope,
            range: Range<usize>,
        ) -> Vec<HighlightSpan> {
            let text = state.downcast_ref::<String>().unwrap();
            text.match_indices('!')
                .map(|(ix, _)| ix..ix + 1)
                .filter(|r| r.start < range.end && r.end > range.start)
                .map(|range| HighlightSpan {
                    range,
                    highlight: HighlightId::intern("keyword"),
                })
                .collect()
        }
    }

    let registry = Arc::new(LanguageRegistry::new());
    registry.register(Language::new("bang", "Bang").with_highlighter(|_, _| Arc::new(Bangs)));
    cx.update(|cx| gpui_monaco::set_language_registry(cx, registry));
    let model = cx.new(|_| TextModel::new("a!b\nc!").with_language("bang"));
    model.update(cx, |model, cx| model.ensure_syntax(cx));
    cx.run_until_parked();
    model.update(cx, |model, cx| model.edit([(0..0, "!")], cx).unwrap());
    cx.run_until_parked();
    model.read_with(cx, |model, _| {
        assert_eq!(model.highlight_name_at(0).as_deref(), Some("keyword"));
        assert_eq!(model.highlight_name_at(1).as_deref(), None);
        assert_eq!(model.highlight_name_at(2).as_deref(), Some("keyword"));
        assert_eq!(
            model.string_and_comment_ranges(0..7),
            Vec::<Range<usize>>::new()
        );
    });
}

#[test]
fn vscode_theme_reads_bracket_minimap_and_sticky_colors() {
    use gpui_monaco::theme::parse_hex_color;
    let theme = gpui_monaco::Theme::from_vscode_json(
        r##"{
            "type": "dark",
            "colors": {
                "editor.background": "#101010",
                "editorBracketMatch.background": "#112233",
                "editorBracketMatch.border": "#445566",
                "minimap.background": "#202020",
                "minimapSlider.background": "#30303080",
                "editorStickyScroll.background": "#404040"
            }
        }"##,
    )
    .unwrap();
    let c = |hex| parse_hex_color(hex).unwrap();
    assert_eq!(theme.colors.bracket_match_background, c("#112233"));
    assert_eq!(theme.colors.bracket_match_border, c("#445566"));
    assert_eq!(theme.colors.minimap_background, Some(c("#202020")));
    assert_eq!(theme.colors.minimap_slider, c("#30303080"));
    assert_eq!(theme.colors.sticky_scroll_background, Some(c("#404040")));
    assert_eq!(theme.colors.gutter_background, c("#101010"));

    // A defined theme keeps its base's own gutter color.
    let base = gpui_monaco::Theme::from_vscode_json(
        r##"{ "colors": { "editor.background": "#101010", "editorGutter.background": "#0a0a0a" } }"##,
    )
    .unwrap();
    let defined = gpui_monaco::Theme::define(
        "derived",
        &base,
        true,
        Vec::new(),
        &[("editor.background".to_string(), "#151515".to_string())].into(),
    );
    assert_eq!(defined.colors.gutter_background, c("#0a0a0a"));
}
