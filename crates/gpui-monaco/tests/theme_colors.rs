//! Decorations and widgets drawn with theme colors: high contrast outlines,
//! marker severities and language-specific token rules.

use std::sync::Arc;

use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use gpui_monaco::syntax::HighlightId;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{
    CodeEditor, Marker, MarkerSeverity, OverviewRulerLane, TextModel, Theme, theme::parse_hex_color,
};

fn setup<'a>(
    text: &str,
    theme: Theme,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text));
        CodeEditor::new(model, window, cx)
    });
    editor.update(cx, |editor, cx| editor.set_theme(Arc::new(theme), cx));
    cx.run_until_parked();
    (editor, cx)
}

fn find(editor: &Entity<CodeEditor>, query: &str, cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| {
        e.start_find_model_for_tests(cx);
        let mut options = e.find_options().clone();
        options.query = query.into();
        e.change_find_options(options, false, cx);
    });
    cx.run_until_parked();
}

#[gpui::test]
fn high_contrast_find_matches_are_outlined(cx: &mut TestAppContext) {
    let (editor, cx) = setup("hello hello hello", Theme::hc_black(), cx);
    find(&editor, "hello", cx);
    let owner = editor.entity_id();
    let decorations = editor.update(cx, |e, cx| {
        e.model().update(cx, |m, _| m.all_decorations(Some(owner)))
    });
    let outlined: Vec<_> = decorations
        .iter()
        .filter(|d| d.options.border.is_some())
        .collect();
    assert_eq!(outlined.len(), 3);
    // Monaco's hc-black outlines matches in `contrastActiveBorder` and
    // doesn't fill them.
    for decoration in outlined {
        assert_eq!(decoration.options.border, parse_hex_color("#F38518"));
        assert!(decoration.options.background.is_none_or(|bg| bg.a == 0.));
    }
}

#[gpui::test]
fn find_matches_are_filled_outside_high_contrast(cx: &mut TestAppContext) {
    let (editor, cx) = setup("hello hello", Theme::vs(), cx);
    find(&editor, "hello", cx);
    let owner = editor.entity_id();
    let decorations = editor.update(cx, |e, cx| {
        e.model().update(cx, |m, _| m.all_decorations(Some(owner)))
    });
    let colors = Theme::vs().colors;
    assert!(decorations.iter().all(|d| d.options.border.is_none()));
    assert!(decorations.iter().any(|d| {
        d.options.background == Some(colors.find_match_highlight_background)
            || d.options.background == Some(colors.find_match_background)
    }));
}

#[gpui::test]
fn markers_carry_their_severity_for_the_theme(cx: &mut TestAppContext) {
    let (editor, cx) = setup("one two three", Theme::vs(), cx);
    let marker = |severity, start: u32| Marker {
        severity,
        message: "problem".into(),
        range: PointUtf16::new(0, start)..PointUtf16::new(0, start + 3),
        ..Default::default()
    };
    editor.update(cx, |e, cx| {
        e.model().update(cx, |m, cx| {
            m.set_markers(
                "test",
                vec![
                    marker(MarkerSeverity::Error, 0),
                    marker(MarkerSeverity::Info, 4),
                    marker(MarkerSeverity::Hint, 8),
                ],
                cx,
            )
        })
    });
    let decorations = editor.update(cx, |e, cx| {
        e.model().update(cx, |m, _| m.all_decorations(None))
    });
    let mut severities: Vec<_> = decorations
        .iter()
        .map(|d| {
            (
                d.options.severity,
                d.options
                    .overview_ruler
                    .as_ref()
                    .map(|ruler| ruler.position),
            )
        })
        .collect();
    severities.sort_by_key(|(severity, _)| *severity);
    assert_eq!(
        severities,
        [
            (Some(MarkerSeverity::Hint), None),
            // Monaco marks infos in the overview ruler too.
            (Some(MarkerSeverity::Info), Some(OverviewRulerLane::Right)),
            (Some(MarkerSeverity::Error), Some(OverviewRulerLane::Right)),
        ]
    );
    // The editor draws them in its theme's colors: vs's, not vs-dark's.
    let colors = editor.read_with(cx, |e, _| e.colors().clone());
    assert_eq!(
        Some(colors.marker_color(MarkerSeverity::Error)),
        parse_hex_color("#E51400")
    );
    assert_eq!(
        colors.marker_overview_color(MarkerSeverity::Info),
        parse_hex_color("#0063D3")
    );
}

#[test]
fn json_keywords_use_monacos_json_rules() {
    let registry = gpui_monaco_languages::registry();
    let theme = Theme::vs();
    let colorized =
        gpui_monaco::colorize::colorize_highlights("{\"a\": true}", "json", &theme, &registry);
    let color_at = |offset: usize| {
        colorized
            .iter()
            .find(|(range, _)| range.contains(&offset))
            .and_then(|(_, style)| style.color)
    };
    // `string.key.json`, then `keyword.json`.
    assert_eq!(color_at(1), parse_hex_color("#A31515"));
    assert_eq!(color_at(6), parse_hex_color("#0451A5"));
    // Without the language, `true` is a plain `keyword`.
    assert_eq!(
        theme
            .style_for(HighlightId::intern("constant.builtin"))
            .foreground,
        parse_hex_color("#0000FF")
    );
}
