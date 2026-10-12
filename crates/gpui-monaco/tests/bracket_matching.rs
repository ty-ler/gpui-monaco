//! Monaco's `contrib/bracketMatching/test/browser/bracketMatching.test.ts`
//! (Go to Bracket, Select to Bracket and Remove Brackets), and GPUI tests
//! for the bracket highlights.

use gpui::{AppContext as _, Entity, Focusable, TestAppContext, VisualTestContext};
use gpui_monaco::cursor::Selection;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, TextModel};

fn p(line: u32, column: u32) -> PointUtf16 {
    PointUtf16::from_monaco(line, column)
}

/// Monaco `new Selection(startLine, startColumn, endLine, endColumn)`: from
/// the anchor to the head.
fn sel(anchor_line: u32, anchor_column: u32, head_line: u32, head_column: u32) -> Selection {
    Selection::new(p(anchor_line, anchor_column), p(head_line, head_column))
}

/// Monaco `createCodeEditorWithBrackets`: a language with the `{}`, `[]`
/// and `()` brackets. Rust's has just those; a configuration without a
/// grammar isn't applied yet (#326). The texts have no strings or
/// comments.
fn editor_with_brackets<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
    });
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_language("rust"));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

fn set_position(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, line: u32, column: u32) {
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(line, column))], cx)
    });
}

fn set_selections(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, s: &[Selection]) {
    editor.update(cx, |e, cx| e.set_selections(s, cx));
}

fn position(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> (u32, u32) {
    editor.read_with(cx, |e, _| e.selections()[0].head.to_monaco())
}

fn selections(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<Selection> {
    editor.read_with(cx, |e, _| e.selections())
}

fn jump(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| e.jump_to_bracket(cx));
}

fn select(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, select_brackets: bool) {
    editor.update(cx, |e, cx| e.select_to_bracket(select_brackets, cx));
}

fn text(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |e, cx| e.model().read(cx).text())
}

/// Monaco: issue #183: jump to matching bracket position
#[gpui::test]
fn jump_to_matching_bracket_position(cx: &mut TestAppContext) {
    let (editor, cx) = editor_with_brackets("var x = (3 + (5-7)) + ((5+3)+5);", cx);

    // start on closing bracket
    set_position(&editor, cx, 1, 20);
    jump(&editor, cx);
    assert_eq!(position(&editor, cx), (1, 9));
    jump(&editor, cx);
    assert_eq!(position(&editor, cx), (1, 19));
    jump(&editor, cx);
    assert_eq!(position(&editor, cx), (1, 9));

    // start on opening bracket
    set_position(&editor, cx, 1, 23);
    jump(&editor, cx);
    assert_eq!(position(&editor, cx), (1, 31));
    jump(&editor, cx);
    assert_eq!(position(&editor, cx), (1, 23));
    jump(&editor, cx);
    assert_eq!(position(&editor, cx), (1, 31));
}

/// Monaco: Jump to next bracket
#[gpui::test]
fn jump_to_next_bracket(cx: &mut TestAppContext) {
    let (editor, cx) = editor_with_brackets("var x = (3 + (5-7)); y();", cx);

    // start position between brackets
    set_position(&editor, cx, 1, 16);
    jump(&editor, cx);
    assert_eq!(position(&editor, cx), (1, 18));
    jump(&editor, cx);
    assert_eq!(position(&editor, cx), (1, 14));
    jump(&editor, cx);
    assert_eq!(position(&editor, cx), (1, 18));

    // skip brackets in comments
    set_position(&editor, cx, 1, 21);
    jump(&editor, cx);
    assert_eq!(position(&editor, cx), (1, 23));
    jump(&editor, cx);
    assert_eq!(position(&editor, cx), (1, 24));
    jump(&editor, cx);
    assert_eq!(position(&editor, cx), (1, 23));

    // do not break if no brackets are available
    set_position(&editor, cx, 1, 26);
    jump(&editor, cx);
    assert_eq!(position(&editor, cx), (1, 26));
}

/// Monaco: Select to next bracket
#[gpui::test]
fn select_to_next_bracket(cx: &mut TestAppContext) {
    let (editor, cx) = editor_with_brackets("var x = (3 + (5-7)); y();", cx);

    // start position in open brackets
    set_position(&editor, cx, 1, 9);
    select(&editor, cx, true);
    assert_eq!(position(&editor, cx), (1, 20));
    assert_eq!(selections(&editor, cx), vec![sel(1, 9, 1, 20)]);

    // start position in close brackets (should select backwards)
    set_position(&editor, cx, 1, 20);
    select(&editor, cx, true);
    assert_eq!(position(&editor, cx), (1, 9));
    assert_eq!(selections(&editor, cx), vec![sel(1, 20, 1, 9)]);

    // start position between brackets
    set_position(&editor, cx, 1, 16);
    select(&editor, cx, true);
    assert_eq!(position(&editor, cx), (1, 19));
    assert_eq!(selections(&editor, cx), vec![sel(1, 14, 1, 19)]);

    // start position outside brackets
    set_position(&editor, cx, 1, 21);
    select(&editor, cx, true);
    assert_eq!(position(&editor, cx), (1, 25));
    assert_eq!(selections(&editor, cx), vec![sel(1, 23, 1, 25)]);

    // do not break if no brackets are available
    set_position(&editor, cx, 1, 26);
    select(&editor, cx, true);
    assert_eq!(position(&editor, cx), (1, 26));
    assert_eq!(selections(&editor, cx), vec![sel(1, 26, 1, 26)]);
}

const ENCLOSING_TEXT: &str =
    "const x = {\n    something: [0, 1, 2],\n    another: true,\n    somethingmore: [0, 2, 4]\n};";

/// Monaco: issue #1772: jump to enclosing brackets
#[gpui::test]
fn jump_to_enclosing_brackets(cx: &mut TestAppContext) {
    let (editor, cx) = editor_with_brackets(ENCLOSING_TEXT, cx);
    set_position(&editor, cx, 3, 5);
    jump(&editor, cx);
    assert_eq!(selections(&editor, cx), vec![sel(5, 1, 5, 1)]);
}

/// Monaco: issue #43371: argument to not select brackets
#[gpui::test]
fn argument_to_not_select_brackets(cx: &mut TestAppContext) {
    let (editor, cx) = editor_with_brackets(ENCLOSING_TEXT, cx);
    set_position(&editor, cx, 3, 5);
    select(&editor, cx, false);
    assert_eq!(selections(&editor, cx), vec![sel(1, 12, 5, 1)]);
}

/// Monaco: issue #45369: Select to Bracket with multicursor
#[gpui::test]
fn select_to_bracket_with_multicursor(cx: &mut TestAppContext) {
    let (editor, cx) = editor_with_brackets("{  }   {   }   { }", cx);

    // cursors inside brackets become selections of the entire bracket contents
    set_selections(
        &editor,
        cx,
        &[sel(1, 3, 1, 3), sel(1, 10, 1, 10), sel(1, 17, 1, 17)],
    );
    select(&editor, cx, true);
    assert_eq!(
        selections(&editor, cx),
        vec![sel(1, 1, 1, 5), sel(1, 8, 1, 13), sel(1, 16, 1, 19)]
    );

    // cursors to the left of bracket pairs become selections of the entire pair
    set_selections(
        &editor,
        cx,
        &[sel(1, 1, 1, 1), sel(1, 6, 1, 6), sel(1, 14, 1, 14)],
    );
    select(&editor, cx, true);
    assert_eq!(
        selections(&editor, cx),
        vec![sel(1, 1, 1, 5), sel(1, 8, 1, 13), sel(1, 16, 1, 19)]
    );

    // cursors just right of a bracket pair become selections of the entire pair
    set_selections(
        &editor,
        cx,
        &[sel(1, 5, 1, 5), sel(1, 13, 1, 13), sel(1, 19, 1, 19)],
    );
    select(&editor, cx, true);
    assert_eq!(
        selections(&editor, cx),
        vec![sel(1, 5, 1, 1), sel(1, 13, 1, 8), sel(1, 19, 1, 16)]
    );
}

/// Monaco: Removes brackets
#[gpui::test]
fn removes_brackets(cx: &mut TestAppContext) {
    let (editor, cx) = editor_with_brackets("var x = (3 + (5-7)); y();", cx);
    let remove = |cx: &mut VisualTestContext| editor.update(cx, |e, cx| e.remove_brackets(cx));

    // position before the bracket
    set_position(&editor, cx, 1, 9);
    remove(cx);
    assert_eq!(text(&editor, cx), "var x = 3 + (5-7); y();");
    editor.update(cx, |e, cx| e.set_value("var x = (3 + (5-7)); y();", cx));

    // position between brackets
    set_position(&editor, cx, 1, 16);
    remove(cx);
    assert_eq!(text(&editor, cx), "var x = (3 + 5-7); y();");
    remove(cx);
    assert_eq!(text(&editor, cx), "var x = 3 + 5-7; y();");
    remove(cx);
    assert_eq!(text(&editor, cx), "var x = 3 + 5-7; y();");
}

/// The `selectBrackets` argument through `trigger_with_payload`, and the
/// action's default.
#[gpui::test]
fn select_to_bracket_action_arguments(cx: &mut TestAppContext) {
    let (editor, cx) = editor_with_brackets("f(abc)", cx);
    set_position(&editor, cx, 1, 4);
    cx.update(|window, cx| {
        editor.update(cx, |e, cx| {
            e.trigger_with_payload(
                "editor.action.selectToBracket",
                serde_json::json!({ "selectBrackets": false }),
                window,
                cx,
            )
            .unwrap()
        })
    });
    assert_eq!(selections(&editor, cx), vec![sel(1, 3, 1, 6)]);
    set_position(&editor, cx, 1, 4);
    cx.dispatch_action(gpui_monaco::actions::SelectToBracket::default());
    assert_eq!(selections(&editor, cx), vec![sel(1, 2, 1, 7)]);
}

/// The ranges the editor highlights as matching brackets.
fn highlights(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<(u32, u32)> {
    let id = editor.entity_id();
    let model = editor.read_with(cx, |e, _| e.model().clone());
    model.update(cx, |model, _| {
        let mut ranges: Vec<(u32, u32)> = model
            .all_decorations(Some(id))
            .iter()
            .filter(|d| d.options.background.is_some() && d.options.border.is_some())
            .map(|d| (d.range.start as u32, d.range.end as u32))
            .collect();
        ranges.sort();
        ranges
    })
}

/// Monaco matches brackets for up to 100 cursors (`_recomputeBrackets`).
#[gpui::test]
fn bracket_highlights_for_up_to_a_hundred_cursors(cx: &mut TestAppContext) {
    let text = "()\n".repeat(101);
    let (editor, cx) = editor_with_brackets(&text, cx);
    let cursors = |n: u32| {
        (1..=n)
            .map(|line| sel(line, 1, line, 1))
            .collect::<Vec<_>>()
    };
    set_selections(&editor, cx, &cursors(50));
    cx.run_until_parked();
    assert_eq!(highlights(&editor, cx).len(), 100);
    set_selections(&editor, cx, &cursors(100));
    cx.run_until_parked();
    assert_eq!(highlights(&editor, cx).len(), 200);
    set_selections(&editor, cx, &cursors(101));
    cx.run_until_parked();
    assert!(highlights(&editor, cx).is_empty());
}

/// `editorBracketMatch.foreground` colors the matched brackets (Monaco's
/// `bracket-match-inline`), when the theme sets it.
#[gpui::test]
fn bracket_match_foreground(cx: &mut TestAppContext) {
    let (editor, cx) = editor_with_brackets("(a)", cx);
    set_position(&editor, cx, 1, 1);
    cx.run_until_parked();
    let id = editor.entity_id();
    let model = editor.read_with(cx, |e, _| e.model().clone());
    let styles = |cx: &mut VisualTestContext| {
        model.update(cx, |model, _| {
            model
                .all_decorations(Some(id))
                .iter()
                .map(|d| d.options.text.and_then(|t| t.foreground))
                .collect::<Vec<_>>()
        })
    };
    // vs-dark sets none.
    assert_eq!(styles(cx), vec![None, None]);
    let red = gpui_monaco::theme::parse_hex_color("#ff0000").unwrap();
    let theme = editor.read_with(cx, |e, _| e.theme().clone());
    let theme = gpui_monaco::Theme::define(
        "red-brackets",
        &theme,
        true,
        Vec::new(),
        &[(
            "editorBracketMatch.foreground".to_string(),
            "#ff0000".to_string(),
        )]
        .into(),
    );
    editor.update(cx, |e, cx| e.set_theme(std::sync::Arc::new(theme), cx));
    set_position(&editor, cx, 1, 4);
    cx.run_until_parked();
    assert_eq!(styles(cx), vec![Some(red), Some(red)]);
}
