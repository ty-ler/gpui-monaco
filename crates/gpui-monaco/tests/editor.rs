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
    let state = editor.read_with(cx, |e, _| e.save_view_state());
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

fn rust_editor<'a>(
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
