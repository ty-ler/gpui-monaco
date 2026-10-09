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
