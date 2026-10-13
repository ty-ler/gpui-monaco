//! Typing in embedded languages and the bracket configuration (#481):
//! onEnter rules, indentation rules and electric characters of the
//! language at the cursor (Monaco `getEnterAction`, `getIndentForEnter`,
//! `getIndentActionForType`, `getPrecedingValidLine` and
//! `CursorConfiguration.onElectricCharacter`), and bracket pairs over
//! `brackets` and `colorizedBracketPairs` together.

use std::sync::Arc;

use gpui::{AppContext as _, Entity, Focusable, TestAppContext, VisualTestContext};
use gpui_monaco::cursor::{AutoIndent, Selection};
use gpui_monaco::syntax::{Language, LanguageConfiguration, LanguageRegistry};
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, TextModel};
use serde_json::json;

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

fn config(value: serde_json::Value) -> LanguageConfiguration {
    serde_json::from_value(value).unwrap()
}

/// Registers `outer`, which embeds `inner` from a `<` to the next `>` (a
/// Monarch `nextEmbedded`; `<` and `>` are `outer`'s), across lines.
fn embed(outer: LanguageConfiguration, inner: LanguageConfiguration, cx: &mut TestAppContext) {
    let registry = Arc::new(LanguageRegistry::new());
    registry.register(Language::new("outer", "Outer").with_config(outer));
    registry.register(
        Language::new("inner", "Inner")
            .with_config(inner)
            .with_monarch_grammar(&json!({ "tokenizer": { "root": [["\\d+", "number"]] } }))
            .unwrap(),
    );
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, registry);
        // Dropping the `Disposable` keeps the provider.
        let _provider = gpui_monaco::set_monarch_tokens_provider(
            cx,
            "outer",
            &json!({
                "tokenizer": {
                    "root": [
                        ["<", { "token": "tag", "next": "@embedded", "nextEmbedded": "inner" }],
                        ["[^<]", "text"],
                    ],
                    "embedded": [[">", { "token": "tag", "next": "@pop", "nextEmbedded": "@pop" }]],
                },
            }),
        )
        .unwrap();
    });
}

fn editor_for<'a>(
    text: &str,
    language: &'static str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_language(language));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    editor.update(cx, |e, cx| {
        // VS Code's default, for the indentation rules.
        let mut options = e.options().clone();
        options.auto_indent = AutoIndent::Full;
        e.update_options(options, cx);
        e.model().update(cx, |m, cx| m.ensure_syntax(cx))
    });
    cx.run_until_parked();
    (editor, cx)
}

/// The text after typing `typed` at `at` in an editor on `text`.
fn typed(
    text: &str,
    language: &'static str,
    at: PointUtf16,
    typed: &str,
    cx: &mut TestAppContext,
) -> String {
    let (editor, cx) = editor_for(text, language, cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(at)], cx);
        e.type_text(typed, cx);
    });
    cx.run_until_parked();
    editor.read_with(cx, |e, cx| e.model().read(cx).text())
}

fn bundled(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
    });
}

/// Monaco `getEnterAction` uses the onEnter rules of the language at the
/// cursor (`getLanguageIdAtPosition`): the audit's repro, a doc comment in
/// an HTML `<script>`.
#[gpui::test]
fn enter_in_a_script_uses_javascripts_on_enter_rules(cx: &mut TestAppContext) {
    bundled(cx);
    assert_eq!(
        typed("<script>\n  /** doc\n</script>", "html", p(1, 9), "\n", cx),
        "<script>\n  /** doc\n   * \n</script>"
    );
}

/// The embedded language's onEnter rules apply in its region.
#[gpui::test]
fn enter_uses_the_embedded_languages_on_enter_rules(cx: &mut TestAppContext) {
    let comment_continues = config(json!({
        "onEnterRules": [{ "beforeText": "^\\s*//.*$", "action": { "indent": "none", "appendText": "// " } }],
    }));
    embed(LanguageConfiguration::default(), comment_continues, cx);
    assert_eq!(
        typed("a <\n  // note\n>", "outer", p(1, 9), "\n", cx),
        "a <\n  // note\n  // \n>"
    );
}

/// Outside the embedded region, the outer language's onEnter rules apply.
#[gpui::test]
fn enter_outside_the_embedded_region(cx: &mut TestAppContext) {
    let comment_continues = config(json!({
        "onEnterRules": [{ "beforeText": "//", "action": { "indent": "none", "appendText": "// " } }],
    }));
    embed(LanguageConfiguration::default(), comment_continues, cx);
    assert_eq!(
        typed("  // note <>", "outer", p(0, 9), "\n", cx),
        "  // note\n   <>"
    );
}

fn indentation_rules() -> LanguageConfiguration {
    config(json!({
        "indentationRules": { "increaseIndentPattern": "\\{\\s*$", "decreaseIndentPattern": "\\}" },
    }))
}

/// Monaco `getIndentActionForType` leaves a line that mixes languages
/// alone (`isLanguageDifferentFromLineStart`).
#[gpui::test]
fn typing_on_a_line_that_mixes_languages_keeps_its_indentation(cx: &mut TestAppContext) {
    embed(indentation_rules(), indentation_rules(), cx);
    assert_eq!(
        typed("f {\n  <x>", "outer", p(1, 3), "}", cx),
        "f {\n  <}x>"
    );
}

/// On a line in one language, typing re-indents by the indentation rules.
#[gpui::test]
fn typing_on_a_line_in_one_language_reindents(cx: &mut TestAppContext) {
    embed(indentation_rules(), LanguageConfiguration::default(), cx);
    assert_eq!(typed("f {\n  x", "outer", p(1, 2), "}", cx), "f {\n}x");
}

fn rules_ignoring_hashes() -> LanguageConfiguration {
    config(json!({
        "indentationRules": {
            "increaseIndentPattern": "\\{",
            "decreaseIndentPattern": "^\\s*\\}",
            "unIndentedLinePattern": "^#",
        },
    }))
}

/// Monaco `getPrecedingValidLine` stops at a line starting in another
/// language, so Enter in an embedded region doesn't inherit indentation
/// from the outer language's lines above it.
#[gpui::test]
fn indentation_isnt_inherited_across_an_embedded_languages_boundary(cx: &mut TestAppContext) {
    embed(rules_ignoring_hashes(), rules_ignoring_hashes(), cx);
    assert_eq!(typed("x {<\n#c", "outer", p(1, 2), "\n", cx), "x {<\n#c\n");
}

/// Within one language, the ignored line is skipped.
#[gpui::test]
fn indentation_is_inherited_over_ignored_lines(cx: &mut TestAppContext) {
    embed(
        rules_ignoring_hashes(),
        LanguageConfiguration::default(),
        cx,
    );
    assert_eq!(
        typed("x {\n#c", "outer", p(1, 2), "\n", cx),
        "x {\n#c\n    "
    );
}

fn square_and_round() -> LanguageConfiguration {
    config(json!({ "brackets": [["[", "]"], ["(", ")"]] }))
}

/// Monaco `CursorConfiguration.onElectricCharacter` lines a closing
/// bracket up by the brackets of the language at the cursor, though which
/// characters are electric is the model's language's.
#[gpui::test]
fn electric_characters_use_the_embedded_languages_brackets(cx: &mut TestAppContext) {
    embed(
        square_and_round(),
        config(json!({ "brackets": [["(", ")"]] })),
        cx,
    );
    assert_eq!(typed("<[\n    ", "outer", p(1, 4), "]", cx), "<[\n    ]");
}

/// A closing bracket of the embedded language is lined up.
#[gpui::test]
fn electric_characters_in_an_embedded_region(cx: &mut TestAppContext) {
    embed(
        square_and_round(),
        config(json!({ "brackets": [["(", ")"]] })),
        cx,
    );
    assert_eq!(typed("<(\n    ", "outer", p(1, 4), ")", cx), "<(\n)");
}

/// Outside embedded regions, the model's brackets apply.
#[gpui::test]
fn electric_characters_outside_embedded_regions(cx: &mut TestAppContext) {
    embed(square_and_round(), LanguageConfiguration::default(), cx);
    assert_eq!(typed("[\n    ", "outer", p(1, 4), "]", cx), "[\n]");
}

/// Monaco's bracket pair tree pairs `brackets` and `colorizedBracketPairs`
/// together, so the bundled HTML configuration's empty
/// `colorizedBracketPairs` still pairs its `{}` for
/// `getBracketPairsInRange` and the bracket pair guides.
#[gpui::test]
fn bracket_pairs_include_brackets_that_arent_colorized(cx: &mut TestAppContext) {
    bundled(cx);
    let (editor, cx) = editor_for("<p>\n{\n  x\n}\n</p>", "html", cx);
    let pairs = editor.read_with(cx, |e, cx| {
        e.model()
            .read(cx)
            .bracket_pairs_in_range(p(0, 0)..p(4, 0))
            .into_iter()
            .map(|pair| (pair.open, pair.close))
            .collect::<Vec<_>>()
    });
    assert_eq!(pairs, vec![(p(1, 0)..p(1, 1), p(3, 0)..p(3, 1))]);
}

/// Without `colorizedBracketPairs`, every bracket pair but `<>` is
/// colorized: a closed `<>` is neither colored nor nests the brackets in
/// it, but still pairs.
#[gpui::test]
fn angle_brackets_arent_colorized_by_default(cx: &mut TestAppContext) {
    let registry = Arc::new(LanguageRegistry::new());
    registry.register(Language::new("angles", "Angles").with_config(config(json!({
        "brackets": [["<", ">"], ["(", ")"]],
    }))));
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, registry);
    });
    let (editor, cx) = editor_for("Vec<(a)>", "angles", cx);
    let (colors, ranges) = editor.read_with(cx, |e, cx| {
        (e.colors().clone(), e.bracket_colors_for_tests(cx))
    });
    let level = |l: u32| colors.bracket_color(l).unwrap();
    assert_eq!(ranges, vec![(4..5, level(0)), (6..7, level(0))]);
    let pairs = editor.read_with(cx, |e, cx| {
        e.model()
            .read(cx)
            .bracket_pairs_in_range(p(0, 0)..p(0, 8))
            .into_iter()
            .map(|pair| (pair.open.start.column, pair.level))
            .collect::<Vec<_>>()
    });
    assert_eq!(pairs, vec![(3, 0), (4, 1)]);
}
