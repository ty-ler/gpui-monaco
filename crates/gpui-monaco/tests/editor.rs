//! Integration tests for `CodeEditor` running in a GPUI test window: input
//! routing, keymap, model synchronisation, undo cursor restore, clipboard and
//! IME composition.

use gpui::{
    AppContext as _, ClipboardItem, Entity, EntityInputHandler, Focusable, TestAppContext,
    VisualTestContext,
};
use gpui_monaco::cursor::Selection;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, EditorOptions, TextModel};

fn p(line: u32, column: u32) -> PointUtf16 {
    PointUtf16::from_monaco(line, column)
}

fn editor_with_text<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| CodeEditor::with_text(&text, window, cx));
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

fn text(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |editor, cx| editor.model().read(cx).text())
}

fn heads(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<(u32, u32)> {
    editor.read_with(cx, |editor, _| {
        editor
            .selections()
            .iter()
            .map(|s| s.head.to_monaco())
            .collect()
    })
}

#[gpui::test]
fn typing_and_keymap(cx: &mut TestAppContext) {
    let (editor, cx) = editor_with_text("", cx);
    cx.simulate_input("hello");
    assert_eq!(text(&editor, cx), "hello");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("world");
    assert_eq!(text(&editor, cx), "hello\nworld");
    cx.simulate_keystrokes("left left backspace");
    assert_eq!(text(&editor, cx), "hello\nwold");
    assert_eq!(heads(&editor, cx), vec![(2, 3)]);
    cx.simulate_keystrokes("up");
    assert_eq!(heads(&editor, cx), vec![(1, 3)]);
    cx.simulate_keystrokes("secondary-a backspace");
    assert_eq!(text(&editor, cx), "");
}

#[gpui::test]
fn undo_groups_typing_and_restores_cursor(cx: &mut TestAppContext) {
    let (editor, cx) = editor_with_text("x", cx);
    cx.simulate_keystrokes("secondary-right");
    cx.simulate_input("abc");
    cx.simulate_keystrokes("left");
    cx.simulate_input("Z");
    assert_eq!(text(&editor, cx), "xabZc");
    cx.simulate_keystrokes("secondary-z");
    assert_eq!(text(&editor, cx), "xabc");
    assert_eq!(heads(&editor, cx), vec![(1, 4)]);
    cx.simulate_keystrokes("secondary-z");
    assert_eq!(text(&editor, cx), "x");
    assert_eq!(heads(&editor, cx), vec![(1, 2)]);
    redo(&editor, cx);
    assert_eq!(text(&editor, cx), "xabc");
}

fn redo(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    editor.update(cx, |editor, cx| editor.redo(cx));
    cx.run_until_parked();
}

#[gpui::test]
fn trim_auto_whitespace(cx: &mut TestAppContext) {
    let (editor, cx) = editor_with_text("    x", cx);
    cx.simulate_keystrokes("end enter enter");
    // The next edit removed the indentation Enter inserted on line 2.
    assert_eq!(text(&editor, cx), "    x\n\n    ");
    assert_eq!(heads(&editor, cx), vec![(3, 5)]);
    cx.simulate_keystrokes("secondary-z");
    assert_eq!(text(&editor, cx), "    x\n    ");

    let model = editor.read_with(cx, |editor, _| editor.model().clone());
    model.update(cx, |model, cx| {
        let options = gpui_monaco::ModelOptions {
            trim_auto_whitespace: false,
            ..model.options().clone()
        };
        model.update_options(options, cx);
    });
    cx.simulate_keystrokes("enter");
    assert_eq!(text(&editor, cx), "    x\n    \n    ");
}

#[gpui::test]
fn two_editors_share_a_model(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let model = cx.new(|_| TextModel::new("one\ntwo"));
    let model_for_window = model.clone();
    let (first, cx) =
        cx.add_window_view(|window, cx| CodeEditor::new(model_for_window, window, cx));
    let second = cx.update(|window, cx| cx.new(|cx| CodeEditor::new(model.clone(), window, cx)));

    // Put the second editor's cursor at the start of "two".
    second.update(cx, |editor, cx| {
        editor.set_selections(&[Selection::collapsed(p(2, 1))], cx)
    });
    cx.update(|window, cx| window.focus(&first.focus_handle(cx), cx));
    cx.run_until_parked();
    cx.simulate_input(">> ");
    cx.run_until_parked();
    assert_eq!(model.read_with(cx, |m, _| m.text()), ">> one\ntwo");
    // Edits on another row don't move it.
    assert_eq!(
        second.read_with(cx, |e, _| e.selections()[0].head.to_monaco()),
        (2, 1)
    );
    cx.simulate_keystrokes("enter");
    assert_eq!(model.read_with(cx, |m, _| m.text()), ">> \none\ntwo");
    assert_eq!(
        second.read_with(cx, |e, _| e.selections()[0].head.to_monaco()),
        (3, 1)
    );
}

#[gpui::test]
fn copy_empty_selection_pastes_full_line_above(cx: &mut TestAppContext) {
    let (editor, cx) = editor_with_text("first\nsecond", cx);
    cx.simulate_keystrokes("right secondary-c down");
    let clipboard = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(clipboard.as_deref(), Some("first\n"));
    cx.simulate_keystrokes("secondary-v");
    assert_eq!(text(&editor, cx), "first\nfirst\nsecond");
}

#[gpui::test]
fn plain_clipboard_paste_replaces_selection(cx: &mut TestAppContext) {
    let (editor, cx) = editor_with_text("abc", cx);
    cx.write_to_clipboard(ClipboardItem::new_string("XY".into()));
    cx.simulate_keystrokes("shift-right secondary-v");
    assert_eq!(text(&editor, cx), "XYbc");
    assert_eq!(heads(&editor, cx), vec![(1, 3)]);
}

#[gpui::test]
fn ime_composition(cx: &mut TestAppContext) {
    let (editor, cx) = editor_with_text("a", cx);
    cx.simulate_keystrokes("secondary-right");
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.replace_and_mark_text_in_range(None, "に", None, window, cx);
            assert_eq!(editor.marked_text_range(window, cx), Some(1..2));
            editor.replace_and_mark_text_in_range(None, "にほ", None, window, cx);
            assert_eq!(editor.marked_text_range(window, cx), Some(1..3));
            editor.replace_text_in_range(None, "日本", window, cx);
            assert_eq!(editor.marked_text_range(window, cx), None);
        })
    });
    assert_eq!(text(&editor, cx), "a日本");
    assert_eq!(heads(&editor, cx), vec![(1, 4)]);
}

#[gpui::test]
fn read_only_ignores_input(cx: &mut TestAppContext) {
    let (editor, cx) = editor_with_text("keep", cx);
    editor.update(cx, |editor, cx| {
        let options = EditorOptions {
            read_only: true,
            ..editor.options().clone()
        };
        editor.update_options(options, cx);
    });
    cx.simulate_input("x");
    cx.simulate_keystrokes("backspace enter");
    assert_eq!(text(&editor, cx), "keep");
}

#[gpui::test]
fn view_state_round_trip(cx: &mut TestAppContext) {
    let (editor, cx) = editor_with_text("a\nb\nc", cx);
    cx.simulate_keystrokes("down shift-down");
    let state = editor.read_with(cx, |e, cx| e.save_view_state(cx));
    cx.simulate_keystrokes("secondary-up");
    assert_eq!(heads(&editor, cx), vec![(1, 1)]);
    editor.update(cx, |e, cx| e.restore_view_state(&state, cx));
    assert_eq!(heads(&editor, cx), vec![(3, 1)]);
}

#[gpui::test]
fn shift_end_after_merging_multicursor_selections(cx: &mut TestAppContext) {
    let (editor, cx) = editor_with_text("aa\nbb\ncc\n    dddd\nee", cx);
    cx.simulate_keystrokes("secondary-alt-down secondary-alt-down");
    assert_eq!(heads(&editor, cx).len(), 3);
    cx.simulate_keystrokes("down shift-down shift-down");
    cx.simulate_keystrokes("shift-end");
    let selections = editor.read_with(cx, |e, _| e.selections());
    assert!(
        selections
            .iter()
            .all(|s| s.head.column == if s.head.row == 3 { 8 } else { 2 }),
        "{selections:?}"
    );
}

#[gpui::test]
fn folding_hides_rows_and_cursor_skips_them(cx: &mut TestAppContext) {
    let (editor, cx) = editor_with_text("fn a() {\n    one\n    two\n}\nafter", cx);
    // Let the background fold-range computation finish.
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    cx.simulate_keystrokes("down");
    cx.simulate_keystrokes("secondary-alt-[");
    let lines = editor.read_with(cx, |e, _| e.view_lines.line_count());
    assert_eq!(lines, 3, "rows 1-2 hidden");
    // The cursor moved out of the hidden row to the end of the fold start.
    assert_eq!(heads(&editor, cx), vec![(1, 9)]);
    cx.simulate_keystrokes("down");
    assert_eq!(heads(&editor, cx), vec![(4, 2)]);
    // Back up restores the remembered column; up again on the first line
    // goes to column 1, like Monaco.
    cx.simulate_keystrokes("up");
    assert_eq!(heads(&editor, cx), vec![(1, 9)]);
    cx.simulate_keystrokes("up");
    assert_eq!(heads(&editor, cx), vec![(1, 1)]);
    // Typing inside the region's visible start line keeps it folded;
    // unfold restores the rows.
    cx.simulate_keystrokes("secondary-alt-]");
    assert_eq!(editor.read_with(cx, |e, _| e.view_lines.line_count()), 5);
}

#[gpui::test]
fn decorations_follow_edits_and_respect_owner(cx: &mut TestAppContext) {
    use gpui_monaco::DecorationOptions;
    let (editor, cx) = editor_with_text("let value = 1;", cx);
    let model = editor.read_with(cx, |e, _| e.model().clone());
    let editor_id = editor.entity_id();
    let ids = model.update(cx, |model, cx| {
        let shared =
            model.delta_decorations(None, &[], vec![(4..9, DecorationOptions::default())], cx);
        let own = model.delta_decorations(
            Some(editor_id),
            &[],
            vec![(0..3, DecorationOptions::default())],
            cx,
        );
        [shared[0], own[0]]
    });
    cx.simulate_keystrokes("secondary-right");
    cx.simulate_input(" // done");
    cx.simulate_keystrokes("secondary-left");
    cx.simulate_input("  ");
    model.update(cx, |model, _| {
        assert_eq!(model.decoration(ids[0]).unwrap().range, 6..11);
        // Default stickiness grows when typing at the start edge.
        assert_eq!(model.decoration(ids[1]).unwrap().range, 0..5);
        assert_eq!(model.decorations_in_range(0..100, None).len(), 1);
        assert_eq!(model.decorations_in_range(0..100, Some(editor_id)).len(), 2);
    });
}

#[gpui::test]
fn view_zone_follows_its_line(cx: &mut TestAppContext) {
    use gpui_monaco::{ViewZone, ZoneHeight};
    let (editor, cx) = editor_with_text("a\nb\nc", cx);
    let view = cx.update(|_, cx| cx.new(|_| gpui::Empty).into());
    editor.update(cx, |editor, cx| {
        editor.add_view_zone(
            ViewZone {
                after_line: 2,
                height: ZoneHeight::Lines(3.),
                view,
            },
            cx,
        );
    });
    cx.run_until_parked();
    // Clicking below the zone lands on line 3; the zone pushed it down.
    let line_3_top = editor.read_with(cx, |e, _| {
        let layout = e.layout_for_tests().unwrap();
        layout.0
    });
    let line_height = editor.read_with(cx, |e, _| e.layout_for_tests().unwrap().1);
    assert_eq!(line_3_top, line_height * 5.);
    // Typing a newline above moves the zone with line 2.
    cx.simulate_keystrokes("enter");
    let line_4_top = editor.read_with(cx, |e, _| e.layout_for_tests().unwrap().0);
    assert_eq!(line_4_top, line_height * 6.);
}

#[gpui::test]
fn find_and_replace_through_the_widget(cx: &mut TestAppContext) {
    let (editor, cx) = editor_with_text("foo bar\nfoo baz foo\nqux", cx);
    // Cmd-F seeds the query with the word under the cursor.
    cx.simulate_keystrokes("secondary-f");
    cx.run_until_parked();
    let (visible, query) = editor.read_with(cx, |e, _| {
        (e.is_find_visible(), e.find_options().query.clone())
    });
    assert!(visible);
    assert_eq!(query, "foo");
    assert_eq!(
        editor.read_with(cx, |e, _| e.find_status()),
        Some("1 of 3".into())
    );
    // Typing in the (focused) find input replaces the selected seed.
    cx.simulate_input("ba");
    cx.run_until_parked();
    assert_eq!(
        editor.read_with(cx, |e, _| e.find_options().query.clone()),
        "ba"
    );
    assert_eq!(
        editor.read_with(cx, |e, _| e.find_status()),
        Some("1 of 2".into())
    );
    assert_eq!(heads(&editor, cx), vec![(1, 7)]);
    // Enter moves to the next match; shift-enter back.
    cx.simulate_keystrokes("enter");
    assert_eq!(heads(&editor, cx), vec![(2, 7)]);
    cx.simulate_keystrokes("shift-enter");
    assert_eq!(heads(&editor, cx), vec![(1, 7)]);
    // Regex toggle + replace all.
    editor.update(cx, |e, cx| {
        let mut options = e.find_options().clone();
        options.query = "ba(.)".into();
        options.is_regex = true;
        options.replace = "B$1!".into();
        e.set_find_options(options, cx);
        e.replace_all(cx);
    });
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "foo Br!\nfoo Bz! foo\nqux");
    // Escape closes the widget and returns focus to the editor.
    cx.simulate_keystrokes("escape");
    assert!(!editor.read_with(cx, |e, _| e.is_find_visible()));
    cx.simulate_input("!");
    assert!(text(&editor, cx).contains('!'));
}

#[gpui::test]
fn escape_in_editor_closes_find(cx: &mut TestAppContext) {
    let (editor, cx) = editor_with_text("foo bar foo", cx);
    cx.simulate_keystrokes("secondary-f");
    cx.run_until_parked();
    assert!(editor.read_with(cx, |e, _| e.is_find_visible()));
    // Focus the editor again (as by clicking into the text), then Escape.
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.simulate_keystrokes("escape");
    assert!(!editor.read_with(cx, |e, _| e.is_find_visible()));
}

#[gpui::test]
fn read_only_blocks_undo(cx: &mut TestAppContext) {
    let (editor, cx) = editor_with_text("", cx);
    cx.simulate_input("abc");
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.read_only = true;
        e.update_options(options, cx);
    });
    cx.simulate_keystrokes("secondary-z");
    assert_eq!(text(&editor, cx), "abc");
}

#[gpui::test]
fn set_model_drops_decorations_owned_in_the_old_model(cx: &mut TestAppContext) {
    use gpui_monaco::DecorationOptions;
    // Bracket highlights are decorations the editor owns in its model.
    let (editor, cx) = editor_with_text("(a)", cx);
    let old_model = editor.read_with(cx, |e, _| e.model().clone());
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(1, 1))], cx)
    });
    cx.run_until_parked();
    let other = cx.new(|_| TextModel::new("x\ny"));
    let host = other.update(cx, |model, cx| {
        model.delta_decorations(None, &[], vec![(0..1, DecorationOptions::default())], cx)
    });
    editor.update(cx, |e, cx| e.set_model(other.clone(), cx));
    // Moving the cursor updates bracket highlights in the new model; that
    // must not delete the host's decoration with a stale id.
    cx.simulate_keystrokes("right right");
    cx.run_until_parked();
    other.read_with(cx, |model, _| assert!(model.decoration(host[0]).is_some()));
    let editor_id = editor.entity_id();
    old_model.update(cx, |model, _| {
        assert!(model.all_decorations(Some(editor_id)).is_empty())
    });
}

fn rust_editor<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    language_editor(text, "rust", cx)
}

fn language_editor<'a>(
    text: &str,
    language: &'static str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
    });
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_language(language));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

#[gpui::test]
fn toggle_comments_by_language(cx: &mut TestAppContext) {
    let (editor, cx) = rust_editor("fn a() {\n    b();\n}", cx);
    cx.simulate_keystrokes("secondary-a secondary-/");
    assert_eq!(text(&editor, cx), "// fn a() {\n//     b();\n// }");
    cx.simulate_keystrokes("secondary-/");
    assert_eq!(text(&editor, cx), "fn a() {\n    b();\n}");
    cx.simulate_keystrokes("secondary-z");
    assert_eq!(text(&editor, cx), "// fn a() {\n//     b();\n// }");
}

#[gpui::test]
fn toggle_comments_use_the_injected_language(cx: &mut TestAppContext) {
    let (editor, cx) = language_editor("# T\n\n```rust\nlet x = 1;\n```\n\ntext\n", "markdown", cx);
    // Inside the Rust fence: Rust's line comment.
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(4, 1))], cx)
    });
    cx.simulate_keystrokes("secondary-/");
    assert_eq!(
        text(&editor, cx),
        "# T\n\n```rust\n// let x = 1;\n```\n\ntext\n"
    );
    // In Markdown text: Markdown has only block comments.
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(7, 1))], cx)
    });
    cx.simulate_keystrokes("secondary-/");
    assert_eq!(
        text(&editor, cx),
        "# T\n\n```rust\n// let x = 1;\n```\n\n<!-- text -->\n"
    );
}

#[gpui::test]
fn rust_auto_close_and_indent(cx: &mut TestAppContext) {
    let (editor, cx) = rust_editor("", cx);
    cx.simulate_input("fn main() {");
    assert_eq!(text(&editor, cx), "fn main() {}");
    cx.simulate_keystrokes("enter");
    assert_eq!(text(&editor, cx), "fn main() {\n    \n}");
    assert_eq!(heads(&editor, cx), vec![(2, 5)]);
    cx.simulate_input("let s = \"");
    assert_eq!(text(&editor, cx), "fn main() {\n    let s = \"\"\n}");
    // Typing the closing quote overtypes the auto-inserted one.
    cx.simulate_input("x\"");
    assert_eq!(text(&editor, cx), "fn main() {\n    let s = \"x\"\n}");
    // Backspace between an auto-closed pair deletes both.
    cx.simulate_input("(");
    assert_eq!(text(&editor, cx), "fn main() {\n    let s = \"x\"()\n}");
    cx.simulate_keystrokes("backspace");
    assert_eq!(text(&editor, cx), "fn main() {\n    let s = \"x\"\n}");
}

#[gpui::test]
fn bracket_matching_highlights_and_jumps(cx: &mut TestAppContext) {
    let (editor, cx) = rust_editor("fn f() { let v = [1, (2)]; }", cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let model = editor.read_with(cx, |e, _| e.model().clone());
    let editor_id = editor.entity_id();
    // Cursor right after `[`.
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(1, 19))], cx)
    });
    cx.run_until_parked();
    let highlighted: Vec<std::ops::Range<usize>> = model.update(cx, |m, _| {
        m.decorations_in_range(0..100, Some(editor_id))
            .into_iter()
            .map(|d| d.range)
            .collect()
    });
    // `[` is byte 17 and its `]` byte 24.
    assert_eq!(highlighted, vec![17..18, 24..25]);
    // Next to the cursor: marked in the overview ruler, as in Monaco.
    let marked = model.update(cx, |m, _| {
        m.decorations_in_range(0..100, Some(editor_id))
            .iter()
            .all(|d| d.options.overview_ruler.is_some())
    });
    assert!(marked);
    // Monaco's default `matchBrackets: "always"`: inside `[1, (2)]` but next
    // to no bracket, the enclosing pair is highlighted, without the mark.
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(1, 21))], cx)
    });
    let decorations = model.update(cx, |m, _| {
        m.decorations_in_range(0..100, Some(editor_id))
            .into_iter()
            .map(|d| (d.range, d.options.overview_ruler.is_some()))
            .collect::<Vec<_>>()
    });
    assert_eq!(decorations, vec![(17..18, false), (24..25, false)]);
    // `"near"`: only pairs next to the cursor.
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.match_brackets = gpui_monaco::options::MatchBrackets::Near;
        e.update_options(options, cx);
        e.set_selections(&[Selection::collapsed(p(1, 20))], cx);
    });
    let count = model.update(cx, |m, _| {
        m.decorations_in_range(0..100, Some(editor_id)).len()
    });
    assert_eq!(count, 0);
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.match_brackets = gpui_monaco::options::MatchBrackets::Always;
        e.update_options(options, cx);
        e.set_selections(&[Selection::collapsed(p(1, 19))], cx);
    });
    cx.simulate_keystrokes("secondary-shift-\\");
    assert_eq!(heads(&editor, cx), vec![(1, 25)]);
    cx.simulate_keystrokes("secondary-shift-\\");
    assert_eq!(heads(&editor, cx), vec![(1, 18)]);
}

#[gpui::test]
fn sticky_scroll_and_minimap_layout(cx: &mut TestAppContext) {
    let mut text = String::from("fn outer() {\n    fn inner() {\n");
    for i in 0..200 {
        text.push_str(&format!("        let v{i} = {i};\n"));
    }
    text.push_str("    }\n}\n");
    let (editor, cx) = editor_with_text(&text, cx);
    cx.run_until_parked();
    // Scroll into the inner body: both scope headers stick.
    editor.update(cx, |e, cx| {
        e.set_scroll_position(gpui::point(gpui::px(0.), gpui::px(1000.)), cx)
    });
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let (sticky_rows, has_minimap) = editor.read_with(cx, |e, _| e.sticky_rows_for_tests());
    assert_eq!(sticky_rows, vec![0, 1]);
    assert!(has_minimap);
}

#[gpui::test]
fn word_start_end_and_delete_commands(cx: &mut TestAppContext) {
    use gpui_monaco::actions::*;
    let (editor, cx) = editor_with_text("fooBar baz qux", cx);
    cx.dispatch_action(CursorWordEndRight);
    assert_eq!(heads(&editor, cx), vec![(1, 7)]);
    cx.dispatch_action(CursorWordStartRight);
    assert_eq!(heads(&editor, cx), vec![(1, 8)]);
    cx.dispatch_action(CursorWordEndLeftSelect);
    assert_eq!(heads(&editor, cx), vec![(1, 7)]);
    // With an empty selection it deletes the word and the space after it.
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(1, 3))], cx)
    });
    cx.dispatch_action(DeleteInsideWord);
    assert_eq!(text(&editor, cx), "baz qux");
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(1, 4))], cx)
    });
    cx.dispatch_action(DeleteWordStartRight);
    assert_eq!(text(&editor, cx), "bazqux");
    cx.dispatch_action(LineBreakInsert);
    assert_eq!(text(&editor, cx), "baz\nqux");
    assert_eq!(heads(&editor, cx), vec![(1, 4)]);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(2, 4))], cx)
    });
    cx.dispatch_action(DeleteWordPartLeft);
    assert_eq!(text(&editor, cx), "baz\n");
}

#[gpui::test]
fn cursor_undo_and_redo(cx: &mut TestAppContext) {
    use gpui_monaco::actions::*;
    let (editor, cx) = editor_with_text("one\ntwo\nthree", cx);
    cx.simulate_keystrokes("down down end");
    assert_eq!(heads(&editor, cx), vec![(3, 6)]);
    cx.simulate_keystrokes("secondary-u");
    assert_eq!(heads(&editor, cx), vec![(3, 1)]);
    cx.simulate_keystrokes("secondary-u secondary-u");
    assert_eq!(heads(&editor, cx), vec![(1, 1)]);
    cx.dispatch_action(CursorRedo);
    assert_eq!(heads(&editor, cx), vec![(2, 1)]);
    // An edit clears the cursor history.
    cx.simulate_input("x");
    cx.simulate_keystrokes("secondary-u");
    assert_eq!(heads(&editor, cx), vec![(2, 2)]);
}

#[gpui::test]
fn selection_anchor(cx: &mut TestAppContext) {
    let (editor, cx) = editor_with_text("hello world", cx);
    cx.simulate_keystrokes("right right secondary-k secondary-b");
    assert_eq!(
        editor.read_with(cx, |e, cx| e.selection_anchor(cx)),
        Some(p(1, 3))
    );
    cx.simulate_keystrokes("end secondary-k secondary-k");
    let selection = editor.read_with(cx, |e, _| e.selections()[0]);
    assert_eq!((selection.anchor, selection.head), (p(1, 3), p(1, 12)));
    // Selecting consumes the anchor.
    assert_eq!(editor.read_with(cx, |e, cx| e.selection_anchor(cx)), None);
    cx.simulate_keystrokes("secondary-k secondary-b escape");
    assert_eq!(editor.read_with(cx, |e, cx| e.selection_anchor(cx)), None);
}

#[gpui::test]
fn keyboard_column_select_and_scroll_commands(cx: &mut TestAppContext) {
    use gpui_monaco::actions::*;
    let (editor, cx) = editor_with_text("abcd\nabcd\nabcd", cx);
    cx.dispatch_action(CursorColumnSelectRight);
    cx.dispatch_action(CursorColumnSelectRight);
    cx.dispatch_action(CursorColumnSelectDown);
    let selections: Vec<_> = editor.read_with(cx, |e, _| {
        e.selections()
            .iter()
            .map(|s| (s.anchor.to_monaco(), s.head.to_monaco()))
            .collect()
    });
    assert_eq!(selections, vec![((1, 1), (1, 3)), ((2, 1), (2, 3))]);

    cx.dispatch_action(CancelSelection);
    cx.dispatch_action(CursorLineEnd);
    cx.dispatch_action(CursorLineStartSelect);
    let selection = editor.read_with(cx, |e, _| e.selections()[0]);
    assert!(selection.head.column == 0 && !selection.is_empty());

    cx.dispatch_action(ScrollLineDown);
    let top = editor.read_with(cx, |e, _| e.scroll_position().y);
    assert!(top > gpui::px(0.));
    cx.dispatch_action(ScrollEditorTop);
    assert_eq!(
        editor.read_with(cx, |e, _| e.scroll_position().y),
        gpui::px(0.)
    );
}

/// Monaco "Overtype Mode": composition type. The composition is inserted,
/// then overtypes as many characters when it's committed.
#[gpui::test]
fn overtype_mode_typing_and_composition(cx: &mut TestAppContext) {
    let (editor, cx) = editor_with_text("123456789", cx);
    cx.simulate_keystrokes("insert");
    assert!(editor.read_with(cx, |e, _| e.is_overtype()));
    cx.simulate_keystrokes("right right");
    cx.simulate_input("a");
    assert_eq!(text(&editor, cx), "12a456789");
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(1, 5))], cx)
    });
    editor.update_in(cx, |e, window, cx| {
        e.replace_and_mark_text_in_range(None, "セ", None, window, cx)
    });
    assert_eq!(text(&editor, cx), "12a4セ56789");
    editor.update_in(cx, |e, window, cx| {
        e.replace_text_in_range(None, "セ", window, cx)
    });
    assert_eq!(text(&editor, cx), "12a4セ6789");
    cx.simulate_keystrokes("insert");
    cx.simulate_input("b");
    assert_eq!(text(&editor, cx), "12a4セb6789");
}

#[gpui::test]
fn line_and_text_transform_actions(cx: &mut TestAppContext) {
    use gpui_monaco::actions::*;
    let (editor, cx) = editor_with_text("gamma\nalpha\nbeta  \nalpha", cx);
    cx.dispatch_action(SortLinesAscending);
    assert_eq!(text(&editor, cx), "alpha\nalpha\nbeta  \ngamma");
    cx.dispatch_action(DeleteDuplicateLines);
    assert_eq!(text(&editor, cx), "alpha\nbeta  \ngamma");
    cx.simulate_keystrokes("secondary-k secondary-x");
    assert_eq!(text(&editor, cx), "alpha\nbeta\ngamma");
    cx.dispatch_action(ReverseLines);
    assert_eq!(text(&editor, cx), "gamma\nbeta\nalpha");
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(1, 3))], cx)
    });
    cx.dispatch_action(TransformToUppercase);
    assert_eq!(text(&editor, cx), "GAMMA\nbeta\nalpha");
    cx.dispatch_action(JoinLines);
    assert_eq!(text(&editor, cx), "GAMMA beta\nalpha");
    cx.dispatch_action(DuplicateSelection);
    assert_eq!(text(&editor, cx), "GAMMA beta\nGAMMA beta\nalpha");
}

#[gpui::test]
fn reindent_lines_and_selected_lines(cx: &mut TestAppContext) {
    use gpui_monaco::actions::*;
    let (editor, cx) = rust_editor("fn a() {\nif x {\nb();\n}\n}", cx);
    cx.dispatch_action(ReindentLines);
    assert_eq!(
        text(&editor, cx),
        "fn a() {\n    if x {\n        b();\n    }\n}"
    );

    editor.update(cx, |e, cx| {
        e.model().clone().update(cx, |m, cx| {
            m.set_value("fn a() {\n    x();\ny();\n        z();\n}", cx)
        });
    });
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[Selection {
                anchor: p(3, 1),
                head: p(3, 4),
            }],
            cx,
        )
    });
    cx.dispatch_action(ReindentSelectedLines);
    // Only the selected line changes, based on the line above it.
    assert_eq!(
        text(&editor, cx),
        "fn a() {\n    x();\n    y();\n        z();\n}"
    );
}

#[gpui::test]
fn in_place_replace_and_remove_brackets(cx: &mut TestAppContext) {
    use gpui_monaco::actions::*;
    let (editor, cx) = rust_editor("let x = foo(1, true);", cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(1, 13))], cx)
    });
    // Monaco: `inPlaceReplace.up` (+1) is Cmd/Ctrl+Shift+, and `.down` is
    // Cmd/Ctrl+Shift+.
    cx.simulate_keystrokes("secondary-shift-,");
    assert_eq!(text(&editor, cx), "let x = foo(2, true);");
    cx.simulate_keystrokes("secondary-shift-. secondary-shift-.");
    assert_eq!(text(&editor, cx), "let x = foo(0, true);");
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(1, 16))], cx)
    });
    cx.dispatch_action(InPlaceReplaceUp);
    assert_eq!(text(&editor, cx), "let x = foo(0, false);");
    // Inside the parentheses: the enclosing pair goes.
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(1, 14))], cx)
    });
    cx.dispatch_action(RemoveBrackets);
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "let x = foo0, false;");
}

#[gpui::test]
fn bracket_pair_colorization(cx: &mut TestAppContext) {
    let (editor, cx) = rust_editor("fn a() { b(\"(\", [1]); }", cx);
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(200));
    cx.run_until_parked();
    let (colors, ranges) = editor.read_with(cx, |e, cx| {
        (e.colors().clone(), e.bracket_colors_for_tests(cx))
    });
    let level = |l: u32| colors.bracket_color(l).unwrap();
    // The `(` inside the string isn't a bracket.
    assert_eq!(
        ranges,
        vec![
            (4..5, level(0)),
            (5..6, level(0)),
            (7..8, level(0)),
            (10..11, level(1)),
            (16..17, level(2)),
            (18..19, level(2)),
            (19..20, level(1)),
            (22..23, level(0)),
        ]
    );
    // Monaco `independentColorPoolPerBracketType`: each kind counts its
    // own nesting, so the `[` inside `(` gets the first color.
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options
            .bracket_pair_colorization
            .independent_color_pool_per_bracket_type = true;
        e.update_options(options, cx);
    });
    let ranges = editor.read_with(cx, |e, cx| e.bracket_colors_for_tests(cx));
    assert!(
        ranges.iter().all(|(_, color)| *color == level(0)),
        "{ranges:?}"
    );
    // Turning it off removes the colors.
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.bracket_pair_colorization.enabled = false;
        e.update_options(options, cx);
    });
    assert!(
        editor
            .read_with(cx, |e, cx| e.bracket_colors_for_tests(cx))
            .is_empty()
    );
}

#[gpui::test]
fn cursor_options_from_editor_options(cx: &mut TestAppContext) {
    let (editor, cx) = rust_editor("a\n    b", cx);
    // `trim_whitespace_on_delete`: deleting the line break also deletes the
    // next line's indentation.
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.trim_whitespace_on_delete = true;
        e.update_options(options, cx);
        e.set_selections(&[Selection::collapsed(p(1, 2))], cx)
    });
    cx.simulate_keystrokes("delete");
    assert_eq!(text(&editor, cx), "ab");
    // `use_tab_stops: false`: Backspace in indentation deletes one space.
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.use_tab_stops = false;
        e.update_options(options, cx);
        e.model().update(cx, |m, cx| m.set_value("        x", cx));
        e.set_selections(&[Selection::collapsed(p(1, 9))], cx)
    });
    cx.simulate_keystrokes("backspace");
    assert_eq!(text(&editor, cx), "       x");
}

#[gpui::test]
fn insert_final_new_line(cx: &mut TestAppContext) {
    let (editor, cx) = editor_with_text("a\nbc", cx);
    let run = |editor: &Entity<CodeEditor>, cx: &mut VisualTestContext| {
        cx.update(|window, cx| {
            editor.update(cx, |e, cx| {
                e.run_action("editor.action.insertFinalNewLine", window, cx)
            })
        });
        cx.run_until_parked();
    };
    // Monaco keeps the selection, even at the end of the document.
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(2, 3))], cx)
    });
    run(&editor, cx);
    assert_eq!(text(&editor, cx), "a\nbc\n");
    assert_eq!(heads(&editor, cx), vec![(2, 3)]);
    // Monaco inserts one even after an empty last line.
    run(&editor, cx);
    assert_eq!(text(&editor, cx), "a\nbc\n\n");
    // One undo step each.
    cx.simulate_keystrokes("secondary-z");
    assert_eq!(text(&editor, cx), "a\nbc\n");
    // Not in a read-only editor.
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.read_only = true;
        e.update_options(options, cx);
    });
    run(&editor, cx);
    assert_eq!(text(&editor, cx), "a\nbc\n");
}
