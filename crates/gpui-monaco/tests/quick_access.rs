//! Quick access (Monaco's standalone quick access): go to line, go to
//! symbol, the command palette and the help list.

use gpui::{AppContext as _, Entity, Focusable, TestAppContext, VisualTestContext};
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, TextModel};

const RUST: &str = "struct Point {\n    x: i32,\n}\n\nimpl Point {\n    fn len(&self) -> i32 {\n        self.x\n    }\n\n    fn scale(&mut self) {}\n}\n\nfn main() {}\n";

fn setup<'a>(
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

fn rows(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.run_until_parked();
    editor.read_with(cx, |e, cx| e.quick_access_rows_for_tests(cx))
}

fn cursor(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> PointUtf16 {
    editor.read_with(cx, |e, _| e.selections()[0].head)
}

fn open(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> bool {
    editor.read_with(cx, |e, _| e.is_quick_access_open())
}

#[gpui::test]
fn go_to_line(cx: &mut TestAppContext) {
    let (editor, cx) = setup(RUST, cx);
    cx.simulate_keystrokes("ctrl-g");
    assert!(open(&editor, cx));
    assert_eq!(
        rows(&editor, cx),
        vec!["> Type a line number to go to (from 1 to 14)."]
    );
    cx.simulate_input("6");
    assert_eq!(
        rows(&editor, cx),
        vec!["> Press 'Enter' to go to line 6 or enter colon : to add a column number."]
    );
    cx.simulate_input(":5");
    assert_eq!(
        rows(&editor, cx),
        vec!["> Press 'Enter' to go to line 6 at column 5."]
    );
    cx.simulate_keystrokes("enter");
    assert!(!open(&editor, cx));
    assert_eq!(cursor(&editor, cx), PointUtf16::new(5, 4));
    // Monaco `gotoLocation`'s `status`: the line, for screen readers.
    editor.read_with(cx, |e, _| {
        assert_eq!(
            e.last_announcement_for_tests().as_deref(),
            Some("Line 6, column 5:     fn len(&self) -> i32 {")
        );
        assert!(e.last_announcement_is_status_for_tests());
    });
}

#[gpui::test]
fn negative_lines_and_cancel_restores(cx: &mut TestAppContext) {
    let (editor, cx) = setup(RUST, cx);
    cx.simulate_keystrokes("ctrl-g");
    cx.simulate_input("-1");
    assert_eq!(
        rows(&editor, cx),
        vec!["> Press 'Enter' to go to line 14 or enter colon : to add a column number."]
    );
    // Escape: back where it was.
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!open(&editor, cx));
    assert_eq!(cursor(&editor, cx), PointUtf16::new(0, 0));
}

#[gpui::test]
fn go_to_symbol(cx: &mut TestAppContext) {
    let (editor, cx) = setup(RUST, cx);
    // Inside `len`: it's active.
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(
                6, 8,
            ))],
            cx,
        )
    });
    cx.simulate_keystrokes("cmd-shift-o");
    assert_eq!(
        rows(&editor, cx),
        vec![
            // (tree-sitter-rust's tags don't include fields.)
            "[symbols (5)]",
            "Point",
            "Point",
            "> len (Point)",
            "scale (Point)",
            "main"
        ]
    );
    cx.simulate_input("sc");
    assert_eq!(rows(&editor, cx), vec!["[symbols (1)]", "> scale (Point)"]);
    cx.simulate_keystrokes("enter");
    assert_eq!(cursor(&editor, cx), PointUtf16::new(9, 7));
}

#[gpui::test]
fn symbols_by_category_and_help(cx: &mut TestAppContext) {
    let (editor, cx) = setup(RUST, cx);
    cx.simulate_keystrokes("cmd-shift-o");
    cx.simulate_input(":");
    let grouped = rows(&editor, cx);
    assert!(
        grouped.contains(&"[methods (2)]".to_string()),
        "{grouped:?}"
    );
    assert!(
        grouped.contains(&"[functions (1)]".to_string()),
        "{grouped:?}"
    );
    // Deleting the prefix shows the help.
    cx.simulate_keystrokes("backspace backspace");
    assert_eq!(
        rows(&editor, cx),
        vec![
            "> : (Go to Line/Column...)",
            ":: (Go to Offset...)",
            "@ (Go to Symbol...)",
            "@: (Go to Symbol by Category...)",
            "> (Show And Run Commands)",
        ]
    );
    // Choosing one switches to it.
    cx.simulate_keystrokes("enter");
    assert_eq!(
        rows(&editor, cx),
        vec!["> Type a line number to go to (from 1 to 14)."]
    );
}

fn text(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |e, cx| e.model().read(cx).text())
}

/// The rows without their keybindings (they differ per platform).
fn labels(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<String> {
    rows(editor, cx)
        .into_iter()
        .map(|row| match row.find(" [") {
            Some(ix) if !row.starts_with('[') => row[..ix].to_string(),
            _ => row,
        })
        .collect()
}

#[gpui::test]
fn command_palette_runs_actions_and_remembers_them(cx: &mut TestAppContext) {
    let (editor, cx) = setup(RUST, cx);
    cx.simulate_keystrokes("f1");
    assert!(open(&editor, cx));
    let all = rows(&editor, cx);
    // Alphabetical, with keybindings.
    assert_eq!(labels(&editor, cx)[0], "> Add Cursor Above");
    assert!(
        all.iter().any(|row| row.starts_with("Fold All [")),
        "{all:?}"
    );

    // Word starts match: "delete line" matches "Delete Duplicate Lines" too.
    cx.simulate_input("delete line");
    assert_eq!(
        labels(&editor, cx),
        vec!["> Delete Duplicate Lines", "Delete Line"]
    );
    cx.simulate_keystrokes("down enter");
    cx.run_until_parked();
    assert!(!open(&editor, cx));
    assert!(text(&editor, cx).starts_with("    x: i32,\n"));

    // Recently used comes first.
    cx.simulate_keystrokes("f1");
    let rows = labels(&editor, cx);
    assert_eq!(
        rows[..3],
        ["[recently used]", "> Delete Line", "[other commands]"]
    );
    assert_eq!(rows[3], "Add Cursor Above");
    cx.simulate_input("zzz");
    assert_eq!(labels(&editor, cx), vec!["> No matching commands"]);
    cx.simulate_keystrokes("escape");
}

#[gpui::test]
fn host_actions_and_preconditions(cx: &mut TestAppContext) {
    let (editor, cx) = setup(RUST, cx);
    editor.update(cx, |editor, cx| {
        let _ = editor.add_action(
            gpui_monaco::EditorActionDescriptor::new("my.hello", "Say Hello", |editor, _, cx| {
                editor
                    .model()
                    .update(cx, |model, cx| model.set_value("hello", cx));
            })
            .alias("Greet")
            .keybinding("ctrl-alt-h"),
            cx,
        );
        editor.update_options(
            gpui_monaco::options::EditorOptions {
                read_only: true,
                ..editor.options().clone()
            },
            cx,
        );
    });
    cx.simulate_keystrokes("f1");
    cx.simulate_input("greet");
    assert_eq!(labels(&editor, cx), vec!["> Say Hello"]);
    // Read-only editors don't offer editing actions.
    cx.simulate_keystrokes("escape");
    cx.simulate_keystrokes("f1");
    cx.simulate_input("delete line");
    assert_eq!(labels(&editor, cx), vec!["> No matching commands"]);
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    // Its keybinding runs it.
    cx.simulate_keystrokes("ctrl-alt-h");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "hello");
    editor.update(cx, |editor, _| assert!(editor.remove_action("my.hello")));
    let ids: Vec<_> = editor.read_with(cx, |e, cx| {
        e.supported_actions(cx)
            .into_iter()
            .map(|a| a.id.to_string())
            .collect()
    });
    assert!(!ids.contains(&"my.hello".to_string()));
    assert!(ids.contains(&"editor.action.quickCommand".to_string()));
    assert!(!ids.contains(&"editor.action.formatDocument".to_string()));
}

fn scroll(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
) -> (gpui::Pixels, gpui::Pixels, gpui::Pixels) {
    editor
        .read_with(cx, |e, cx| e.quick_access_scroll_for_tests(cx))
        .unwrap()
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
}

/// Monaco's quick input list is a scrolling `List`: the wheel scrolls it
/// (never the editor behind it), and the keyboard keeps the active row in
/// view, so every command can be reached.
#[gpui::test]
fn the_list_scrolls_with_the_wheel_and_follows_the_active_row(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&"fn main() {}\n".repeat(200), cx);
    cx.simulate_keystrokes("f1");
    draw(cx);
    let (top, height, content) = scroll(&editor, cx);
    assert_eq!(top, gpui::px(0.));
    assert!(content > height, "{content:?} {height:?}");
    // The wheel over the list.
    let (_, widgets) = editor.read_with(cx, |e, _| e.widget_bounds_for_tests());
    let quick_input = widgets.last().unwrap().0;
    let over_list = gpui::point(quick_input.center().x, quick_input.bottom() - gpui::px(30.));
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: over_list,
        delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(-100.))),
        ..Default::default()
    });
    draw(cx);
    assert_eq!(scroll(&editor, cx).0, gpui::px(100.));
    assert_eq!(
        editor.read_with(cx, |e, _| e.scroll_position().y),
        gpui::px(0.)
    );
    // Up wraps to the last command, revealed at the bottom.
    cx.simulate_keystrokes("up");
    draw(cx);
    let (top, height, content) = scroll(&editor, cx);
    assert_eq!(top, content - height);
    // Down wraps back to the first, at the top.
    cx.simulate_keystrokes("down");
    draw(cx);
    assert_eq!(scroll(&editor, cx).0, gpui::px(0.));
}

/// Keybindings are labeled as Monaco does ("F1", capitalized keys and
/// macOS modifiers in ⌃⇧⌥⌘ order).
#[gpui::test]
fn keybindings_use_monaco_labels(cx: &mut TestAppContext) {
    let (editor, cx) = setup(RUST, cx);
    cx.simulate_keystrokes("f1");
    let rows = rows(&editor, cx);
    assert!(
        rows.iter().any(|row| row == "Command Palette [F1]"),
        "{rows:?}"
    );
    let rename = if cfg!(target_os = "macos") {
        "Change All Occurrences [⌘F2]"
    } else {
        "Change All Occurrences [Ctrl+F2]"
    };
    assert!(rows.iter().any(|row| row == rename), "{rows:?}");
}

/// The active row's index among the items (separators aren't rows).
fn active_index(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Option<usize> {
    rows(editor, cx)
        .iter()
        .filter(|row| !row.starts_with('['))
        .position(|row| row.starts_with("> "))
}

/// Monaco `List.focusNextPage` / `focusPreviousPage`: Page Down focuses
/// the last fully visible row, and only then scrolls a page; Page Up the
/// same upwards.
#[gpui::test]
fn page_keys_move_like_monaco_lists(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&"fn main() {}\n".repeat(200), cx);
    cx.simulate_keystrokes("f1");
    draw(cx);
    let (_, height, _) = scroll(&editor, cx);
    let row = gpui::px(22.);
    let last_visible = ((height / row).floor() as usize).saturating_sub(1);
    assert_eq!(active_index(&editor, cx), Some(0));
    cx.simulate_keystrokes("pagedown");
    draw(cx);
    assert_eq!(active_index(&editor, cx), Some(last_visible));
    assert_eq!(scroll(&editor, cx).0, gpui::px(0.));
    cx.simulate_keystrokes("pagedown");
    draw(cx);
    let top = scroll(&editor, cx).0;
    assert_eq!(top, height);
    let bottom_row = ((top + height) / row).floor() as usize - 1;
    assert_eq!(active_index(&editor, cx), Some(bottom_row));
    cx.simulate_keystrokes("pageup");
    draw(cx);
    let first_visible = (top / row).ceil() as usize;
    assert_eq!(active_index(&editor, cx), Some(first_visible));
    assert_eq!(scroll(&editor, cx).0, top);
    cx.simulate_keystrokes("pageup");
    draw(cx);
    assert_eq!(scroll(&editor, cx).0, gpui::px(0.));
    assert_eq!(active_index(&editor, cx), Some(0));
}

/// Monaco captures the pointer on the list's slider: dragging it scrolls
/// the list from anywhere in the window, with the arrow, until the release.
#[gpui::test]
fn dragging_the_slider_keeps_the_arrow_everywhere(cx: &mut TestAppContext) {
    use gpui::{CursorStyle, Modifiers, MouseButton, point, px};
    let (editor, cx) = setup(&"fn main() {}\n".repeat(200), cx);
    cx.simulate_keystrokes("f1");
    draw(cx);
    let (_, widgets) = editor.read_with(cx, |e, _| e.widget_bounds_for_tests());
    let quick_input = widgets.last().unwrap().0;
    // The slider shows while the mouse is over the list.
    cx.simulate_mouse_move(
        point(quick_input.center().x, quick_input.bottom() - px(30.)),
        None,
        Modifiers::default(),
    );
    draw(cx);
    let captured =
        |cx: &mut VisualTestContext| editor.read_with(cx, |e, cx| e.captured_cursor_for_tests(cx));
    assert_eq!(captured(cx), None);
    let slider = cx
        .debug_bounds("quick-scrollbar-slider")
        .expect("the slider");
    cx.simulate_mouse_move(slider.center(), None, Modifiers::default());
    cx.simulate_mouse_down(slider.center(), MouseButton::Left, Modifiers::default());
    draw(cx);
    assert_eq!(captured(cx), Some(CursorStyle::Arrow));
    // Far outside the list and the editor's text.
    cx.simulate_mouse_move(
        point(px(5.), slider.center().y + px(80.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    draw(cx);
    assert_eq!(captured(cx), Some(CursorStyle::Arrow));
    assert!(scroll(&editor, cx).0 > px(0.));
    cx.simulate_mouse_up(
        point(px(5.), px(5.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    draw(cx);
    assert_eq!(captured(cx), None);
}

fn setup_language<'a>(
    text: &str,
    language: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
    });
    let text = text.to_string();
    let language = language.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_language(language));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

/// Opens Go to Line, types `input` after the `:` and presses Enter; the
/// cursor's 1-based line and column.
fn go_to(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, input: &str) -> (u32, u32) {
    cx.simulate_keystrokes("ctrl-g");
    cx.simulate_input(input);
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(!open(editor, cx), "{input}");
    let at = cursor(editor, cx);
    (at.row + 1, at.column + 1)
}

/// Monaco `gotoLineQuickAccess.test.ts`, "parsePosition works as
/// expected".
#[gpui::test]
fn parse_position_like_monaco(cx: &mut TestAppContext) {
    let (editor, cx) = setup("line 1\nline 2\nline 3\nline 4\nline 5", cx);
    let cases: &[(&str, u32, u32)] = &[
        // :line
        ("-100", 1, 1),
        ("-5", 1, 1),
        ("-1", 5, 1),
        ("0", 1, 1),
        ("1", 1, 1),
        ("2", 2, 1),
        ("5", 5, 1),
        ("6", 5, 1),
        ("7", 5, 1),
        ("100", 5, 1),
        // :line,column
        ("2:-100", 2, 1),
        ("2:-5", 2, 2),
        ("2:-1", 2, 6),
        ("2:0", 2, 1),
        ("2:1", 2, 1),
        ("2:2", 2, 2),
        ("2:6", 2, 6),
        ("2:7", 2, 7),
        ("2:8", 2, 7),
        ("2:100", 2, 7),
        // ::offset (1-based)
        (":-1000", 1, 1),
        (":-10", 4, 5),
        (":-1", 5, 7),
        (":0", 1, 1),
        (":1", 1, 1),
        (":10", 2, 3),
        (":1000", 5, 7),
        // :line#column, :line,column and spaces
        ("-1#6", 5, 6),
        ("2,4", 2, 4),
        ("  2  :  3  ", 2, 3),
        // `parseInt`: what follows the number is ignored.
        ("3.", 3, 1),
        ("4x:2y", 4, 2),
        (":10.5", 2, 3),
    ];
    for &(input, line, column) in cases {
        assert_eq!(go_to(&editor, cx, input), (line, column), "{input}");
    }
    // ::offset (0-based)
    cx.update(|_, cx| gpui_monaco::quick_access::set_goto_offset_zero_based(true, cx));
    for &(input, line, column) in &[
        (":-1000", 1, 1),
        (":-10", 4, 4),
        (":-1", 5, 6),
        (":0", 1, 1),
        (":1", 1, 2),
        (":10", 2, 4),
        (":1000", 5, 7),
    ] {
        assert_eq!(go_to(&editor, cx, input), (line, column), "{input}");
    }
}

/// Monaco `gotoLineQuickAccess.test.ts`, "parsePosition works with tabs".
#[gpui::test]
fn parse_position_with_tabs_like_monaco(cx: &mut TestAppContext) {
    let (editor, cx) = setup("\tline 1\n\t\tline 2\n\tline 3", cx);
    for &(input, line, column) in &[
        ("1:1", 1, 1),
        ("1:2", 1, 1),
        ("1:3", 1, 1),
        ("1:4", 1, 2),
        ("1:5", 1, 2),
        ("1:6", 1, 3),
        ("1:11", 1, 8),
        ("1:12", 1, 8),
        ("1:-5", 1, 3),
        ("1:-6", 1, 2),
        ("1:-7", 1, 2),
        ("1:-8", 1, 1),
        ("1:-9", 1, 1),
        ("1:-10", 1, 1),
        ("1:-11", 1, 1),
        ("2:1", 2, 1),
        ("2:2", 2, 1),
        ("2:3", 2, 1),
        ("2:4", 2, 2),
        ("2:5", 2, 2),
        ("2:8", 2, 3),
        ("2:9", 2, 3),
        ("2:11", 2, 5),
    ] {
        assert_eq!(go_to(&editor, cx, input), (line, column), "{input}");
    }
}

/// Monaco `gotoLineQuickAccess.ts`: `12.` is read as line 12 (`parseInt`),
/// not rejected.
#[gpui::test]
fn line_numbers_parse_like_parse_int(cx: &mut TestAppContext) {
    let (editor, cx) = setup(RUST, cx);
    cx.simulate_keystrokes("ctrl-g");
    cx.simulate_input("12.");
    assert_eq!(
        rows(&editor, cx),
        vec!["> Press 'Enter' to go to line 12 or enter colon : to add a column number."]
    );
}

fn line_numbers(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
) -> gpui_monaco::options::LineNumbers {
    cx.run_until_parked();
    editor.read_with(cx, |e, _| e.options().line_numbers.clone())
}

/// Monaco `provideWithTextEditor`: Go to Line shows absolute line numbers
/// while it's the provider, then relative ones again.
#[gpui::test]
fn go_to_line_shows_absolute_line_numbers(cx: &mut TestAppContext) {
    use gpui_monaco::options::LineNumbers;
    let (editor, cx) = setup(RUST, cx);
    editor.update(cx, |e, cx| {
        e.update_options_with(|o| o.line_numbers = LineNumbers::Relative, cx)
    });
    cx.simulate_keystrokes("ctrl-g");
    assert_eq!(line_numbers(&editor, cx), LineNumbers::On);
    // Another provider: relative again.
    cx.simulate_keystrokes("backspace");
    cx.simulate_input("@");
    assert_eq!(line_numbers(&editor, cx), LineNumbers::Relative);
    cx.simulate_keystrokes("backspace");
    cx.simulate_input("::");
    assert_eq!(line_numbers(&editor, cx), LineNumbers::On);
    cx.simulate_keystrokes("escape");
    assert_eq!(line_numbers(&editor, cx), LineNumbers::Relative);
    // Accepting restores them too.
    cx.simulate_keystrokes("ctrl-g");
    cx.simulate_input("3");
    assert_eq!(line_numbers(&editor, cx), LineNumbers::On);
    cx.simulate_keystrokes("enter");
    assert_eq!(line_numbers(&editor, cx), LineNumbers::Relative);
    // Other line numbers are left alone.
    editor.update(cx, |e, cx| {
        e.update_options_with(|o| o.line_numbers = LineNumbers::Interval, cx)
    });
    cx.simulate_keystrokes("ctrl-g");
    assert_eq!(line_numbers(&editor, cx), LineNumbers::Interval);
    cx.simulate_keystrokes("escape");
    assert_eq!(line_numbers(&editor, cx), LineNumbers::Interval);
}

/// Monaco `doGetSymbolPicks`: the whole query is tried against the name
/// first, so names with spaces match.
#[gpui::test]
fn symbols_with_spaces_match_the_whole_query(cx: &mut TestAppContext) {
    let (editor, cx) = setup_language(
        "# Getting Started\n\ntext\n\n## Change Log\n",
        "markdown",
        cx,
    );
    cx.simulate_keystrokes("cmd-shift-o");
    cx.simulate_input("getting started");
    assert_eq!(
        rows(&editor, cx),
        vec!["[symbols (1)]", "> Getting Started"]
    );
    cx.simulate_keystrokes("enter");
    assert_eq!(cursor(&editor, cx).row, 0);
}

/// The words after the first still match the container when the whole
/// query doesn't match the name.
#[gpui::test]
fn symbol_words_after_the_first_match_the_container(cx: &mut TestAppContext) {
    let (editor, cx) = setup(RUST, cx);
    cx.simulate_keystrokes("cmd-shift-o");
    cx.simulate_input("len point");
    assert_eq!(rows(&editor, cx), vec!["[symbols (1)]", "> len (Point)"]);
    cx.simulate_input("x");
    assert_eq!(rows(&editor, cx), vec!["> No matching editor symbols"]);
}

/// Monaco `addDecorations`: the preview's overview ruler mark is in the
/// Full lane.
#[gpui::test]
fn previews_mark_the_full_overview_ruler_lane(cx: &mut TestAppContext) {
    let (editor, cx) = setup(RUST, cx);
    cx.simulate_keystrokes("ctrl-g");
    cx.simulate_input("5");
    cx.run_until_parked();
    let lanes: Vec<_> = editor.update(cx, |e, cx| {
        let id = Some(cx.entity_id());
        e.model().update(cx, |model, _| {
            model
                .all_decorations(id)
                .into_iter()
                .filter_map(|d| d.options.overview_ruler.as_ref().map(|o| o.position))
                .collect()
        })
    });
    assert_eq!(
        lanes,
        vec![gpui_monaco::decorations::OverviewRulerLane::Full]
    );
}

/// The quick input is centered on the whole editor, gutter included
/// (Monaco's quick input), not on the text area.
#[gpui::test]
fn the_quick_input_centers_on_the_editor(cx: &mut TestAppContext) {
    let (editor, cx) = setup(RUST, cx);
    cx.simulate_keystrokes("f1");
    draw(cx);
    let (bounds, widgets) = editor.read_with(cx, |e, _| e.widget_bounds_for_tests());
    let quick = widgets.last().expect("the quick input").0;
    let left = quick.left() - bounds.left();
    let right = bounds.right() - quick.right();
    assert!((left - right).abs() < gpui::px(1.), "{left:?} {right:?}");
}

/// The quick input's text box is laid out a line tall, so its text is
/// painted (it was 0px tall under an auto-height parent).
#[gpui::test]
fn the_quick_input_text_box_has_a_line_of_height(cx: &mut TestAppContext) {
    let (editor, cx) = setup(RUST, cx);
    cx.simulate_keystrokes("ctrl-g");
    cx.simulate_input("12");
    draw(cx);
    let bounds = editor
        .read_with(cx, |e, cx| e.quick_access_input_bounds_for_tests(cx))
        .expect("the input was laid out");
    assert_eq!(bounds.size.height, gpui::px(22.));
    assert!(bounds.size.width > gpui::px(100.), "{bounds:?}");
}

/// Symbols with and without their own container.
struct ContainerSymbols;

impl gpui_monaco::providers::DocumentSymbolProvider for ContainerSymbols {
    fn provide_document_symbols(
        &self,
        _: &gpui_monaco::providers::ProviderContext,
        _: &mut gpui::App,
    ) -> gpui::Task<anyhow::Result<Vec<gpui_monaco::providers::DocumentSymbol>>> {
        use gpui_monaco::providers::{DocumentSymbol, SymbolKind};
        let at = |row| PointUtf16::new(row, 0)..PointUtf16::new(row, 3);
        gpui::Task::ready(Ok(vec![
            // A flat `SymbolInformation`-like result.
            DocumentSymbol {
                name: "run".into(),
                kind: SymbolKind::Function,
                range: at(0),
                selection_range: at(0),
                container_name: Some("Server".into()),
                ..Default::default()
            },
            DocumentSymbol {
                name: "Bar".into(),
                kind: SymbolKind::Class,
                range: PointUtf16::new(2, 0)..PointUtf16::new(4, 1),
                selection_range: at(2),
                children: vec![DocumentSymbol {
                    name: "foo".into(),
                    kind: SymbolKind::Method,
                    range: at(3),
                    selection_range: at(3),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ]))
    }
}

fn description_highlights(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
) -> Vec<(String, String)> {
    cx.run_until_parked();
    editor.read_with(cx, |e, cx| {
        e.quick_access_description_highlights_for_tests(cx)
            .into_iter()
            .map(|(description, matched)| (description, matched.concat()))
            .collect()
    })
}

/// Monaco `_flattenDocumentSymbols` and `doGetSymbolPicks`: a symbol's own
/// `containerName` is its description, the words after the first match
/// it, and the matched part is highlighted.
#[gpui::test]
fn go_to_symbol_matches_and_highlights_containers(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::providers::register_document_symbol_provider(
            cx,
            gpui_monaco::providers::LanguageSelector::All,
            ContainerSymbols,
        );
    });
    let (editor, cx) = setup("run\n\nBar {\nfoo\n}\n", cx);
    cx.simulate_keystrokes("cmd-shift-o");
    cx.run_until_parked();
    assert_eq!(
        rows(&editor, cx),
        vec!["[symbols (3)]", "> run (Server)", "Bar", "foo (Bar)"]
    );
    // Nothing typed, nothing highlighted.
    assert_eq!(
        description_highlights(&editor, cx),
        vec![
            ("Server".to_string(), String::new()),
            ("Bar".to_string(), String::new())
        ]
    );
    cx.simulate_input("run Server");
    assert_eq!(rows(&editor, cx), vec!["[symbols (1)]", "> run (Server)"]);
    assert_eq!(
        description_highlights(&editor, cx),
        vec![("Server".to_string(), "Server".to_string())]
    );
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-shift-o");
    cx.simulate_input("foo bar");
    assert_eq!(rows(&editor, cx), vec!["[symbols (1)]", "> foo (Bar)"]);
    assert_eq!(
        description_highlights(&editor, cx),
        vec![("Bar".to_string(), "Bar".to_string())]
    );
}

/// Monaco's Go to Symbol action opens with `ItemActivation.NONE`: with
/// the cursor in no symbol, nothing is active or previewed; typing, or
/// opening `@` from the help, activates the first item.
#[gpui::test]
fn go_to_symbol_outside_symbols_activates_nothing(cx: &mut TestAppContext) {
    let (editor, cx) = setup(RUST, cx);
    // The blank line between `struct Point` and `impl Point`.
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(
                3, 0,
            ))],
            cx,
        )
    });
    cx.simulate_keystrokes("cmd-shift-o");
    assert_eq!(
        rows(&editor, cx),
        vec![
            "[symbols (5)]",
            "Point",
            "Point",
            "len (Point)",
            "scale (Point)",
            "main"
        ]
    );
    // No preview: no range highlight, and the cursor stays.
    let previews = editor.update(cx, |e, cx| {
        let id = Some(cx.entity_id());
        e.model()
            .update(cx, |model, _| model.all_decorations(id).len())
    });
    assert_eq!(previews, 0);
    assert_eq!(cursor(&editor, cx), PointUtf16::new(3, 0));
    // Enter does nothing without an active item.
    cx.simulate_keystrokes("enter");
    assert!(open(&editor, cx));
    // Down focuses the first row.
    cx.simulate_keystrokes("down");
    assert_eq!(active_index(&editor, cx), Some(0));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();

    // Typing activates the first match.
    cx.simulate_keystrokes("cmd-shift-o");
    cx.run_until_parked();
    cx.simulate_input("s");
    assert_eq!(active_index(&editor, cx), Some(0));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();

    // `@` typed after the help: Monaco's default `ItemActivation.FIRST`.
    cx.simulate_keystrokes("f1");
    cx.simulate_keystrokes("backspace");
    cx.simulate_input("@");
    cx.run_until_parked();
    assert_eq!(active_index(&editor, cx), Some(0));
}

/// Monaco's "Toggle Zero-Based Offset" is a `Toggle` with the
/// `index-zero` codicon: 16px, with 1px padding and border.
#[gpui::test]
fn the_zero_based_offset_toggle_is_an_icon_toggle(cx: &mut TestAppContext) {
    let (_editor, cx) = setup(RUST, cx);
    cx.simulate_keystrokes("ctrl-g");
    cx.simulate_input(":");
    draw(cx);
    let toggle = cx
        .debug_bounds("quick-zero-based-offset")
        .expect("the toggle");
    assert_eq!(toggle.size.width, gpui::px(20.));
    assert_eq!(toggle.size.height, gpui::px(20.));
}
