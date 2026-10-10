//! Snippet sessions and the snippet controller (Monaco
//! `snippetSession.test.ts`, `snippetController2.test.ts` and
//! `snippetController2.old.test.ts`, MIT).
//!
//! Positions in these tests are written like Monaco's: 1-based lines and
//! columns, converted by [`s`] and [`p`]. Monaco's `SnippetSession` tests
//! drive the session directly; here they go through the editor, whose
//! controller also leaves snippet mode at the final tab stop or when the
//! selections leave the placeholders, so a few of them stop earlier.

use gpui::{AppContext as _, Entity, Focusable, TestAppContext, VisualTestContext};
use gpui_monaco::cursor::Selection;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, ModelOptions, SnippetInsertOptions, TextModel};

/// Monaco's `Selection(startLine, startColumn, endLine, endColumn)`
/// (1-based; the end is the cursor).
fn s(l1: u32, c1: u32, l2: u32, c2: u32) -> Selection {
    Selection::new(
        PointUtf16::new(l1 - 1, c1 - 1),
        PointUtf16::new(l2 - 1, c2 - 1),
    )
}

/// An empty selection at Monaco's 1-based `(line, column)`.
fn p(line: u32, column: u32) -> Selection {
    s(line, column, line, column)
}

struct Ed<'a> {
    editor: Entity<CodeEditor>,
    cx: &'a mut VisualTestContext,
}

fn setup<'a>(text: &str, cx: &'a mut TestAppContext) -> Ed<'a> {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    Ed { editor, cx }
}

/// Monaco `snippetSession.test.ts` setup: two cursors.
fn session_setup(cx: &mut TestAppContext) -> Ed<'_> {
    let mut ed = setup("function foo() {\n    console.log(a);\n}", cx);
    ed.select(&[p(1, 1), p(2, 5)]);
    ed
}

/// Monaco `snippetController2.test.ts` setup: two cursors.
fn controller_setup(cx: &mut TestAppContext) -> Ed<'_> {
    let mut ed = setup("if\n    $state\nfi", cx);
    ed.select(&[p(1, 1), p(2, 5)]);
    ed
}

const OLD_LINES: &[&str] = &[
    "function test() {",
    "\tvar x = 3;",
    "\tvar arr = [];",
    "\t",
    "}",
];

const OLD_TEMPLATE: &str =
    "for (var ${1:index}; $1 < ${2:array}.length; $1++) {\n\tvar element = $2[$1];\n\t$0\n}";

/// Monaco `snippetController2.old.test.ts` `snippetTest`: tabs.
fn old_setup<'a>(lines: &[&str], cx: &'a mut TestAppContext) -> Ed<'a> {
    let mut ed = setup(&lines.join("\n"), cx);
    ed.set_model_options(false, 4, 4);
    ed
}

fn before(n: u32) -> SnippetInsertOptions {
    SnippetInsertOptions {
        overwrite_before: n,
        ..Default::default()
    }
}

impl Ed<'_> {
    fn insert(&mut self, template: &str) {
        self.insert_with(template, SnippetInsertOptions::default());
    }

    fn insert_with(&mut self, template: &str, options: SnippetInsertOptions) {
        self.editor
            .update(self.cx, |e, cx| e.insert_snippet(template, options, cx));
    }

    fn next(&mut self) {
        self.editor
            .update(self.cx, |e, cx| e.next_snippet_placeholder(cx));
    }

    fn prev(&mut self) {
        self.editor
            .update(self.cx, |e, cx| e.prev_snippet_placeholder(cx));
    }

    fn cancel(&mut self, reset_selection: bool) {
        self.editor
            .update(self.cx, |e, cx| e.cancel_snippet(reset_selection, cx));
    }

    fn undo(&mut self) {
        self.editor.update(self.cx, |e, cx| e.undo(cx));
    }

    fn text(&mut self) -> String {
        self.editor
            .read_with(self.cx, |e, cx| e.model().read(cx).text())
    }

    fn line(&mut self, line: u32) -> String {
        self.text()
            .split('\n')
            .nth(line as usize - 1)
            .unwrap_or_default()
            .to_string()
    }

    fn set_value(&mut self, text: &str) {
        let model = self.editor.read_with(self.cx, |e, _| e.model().clone());
        model.update(self.cx, |m, cx| m.set_value(text, cx));
    }

    fn set_model_options(&mut self, insert_spaces: bool, tab_size: u32, indent_size: u32) {
        let model = self.editor.read_with(self.cx, |e, _| e.model().clone());
        model.update(self.cx, |m, cx| {
            m.update_options(
                ModelOptions {
                    tab_size,
                    indent_size,
                    insert_spaces,
                },
                cx,
            )
        });
    }

    fn select(&mut self, selections: &[Selection]) {
        self.editor
            .update(self.cx, |e, cx| e.set_selections(selections, cx));
    }

    fn selections(&mut self) -> Vec<Selection> {
        self.editor.read_with(self.cx, |e, _| e.selections())
    }

    #[track_caller]
    fn assert_selections(&mut self, expected: &[Selection]) {
        assert_eq!(self.selections(), expected);
    }

    /// `(inSnippetMode, hasPrevTabstop, hasNextTabstop)`.
    fn state(&mut self) -> (bool, bool, bool) {
        self.editor.read_with(self.cx, |e, _| {
            (
                e.in_snippet_mode(),
                e.has_prev_tabstop(),
                e.has_next_tabstop(),
            )
        })
    }

    #[track_caller]
    fn assert_state(&mut self, in_snippet: bool, has_prev: bool, has_next: bool) {
        assert_eq!(self.state(), (in_snippet, has_prev, has_next));
    }

    fn in_snippet(&mut self) -> bool {
        self.state().0
    }

    fn type_text(&mut self, text: &str) {
        self.cx.simulate_input(text);
    }

    fn keys(&mut self, keys: &str) {
        self.cx.simulate_keystrokes(keys);
    }

    /// Monaco `Handler.Cut` on non-empty selections: deletes them.
    fn cut(&mut self) {
        self.keys("backspace");
    }
}

// ----- snippetSession.test.ts -----------------------------------------------

#[gpui::test]
fn normalize_whitespace(cx: &mut TestAppContext) {
    // Monaco calls `SnippetSession.adjustWhitespace` directly; here the
    // adjusted snippet is inserted at the position instead.
    let mut ed = session_setup(cx);
    let original = "function foo() {\n    console.log(a);\n}";
    let cases: &[(Selection, &str, &str)] = &[
        (
            p(1, 1),
            "foo",
            "foofunction foo() {\n    console.log(a);\n}",
        ),
        (
            p(1, 1),
            "foo\rbar",
            "foo\nbarfunction foo() {\n    console.log(a);\n}",
        ),
        (
            p(2, 5),
            "foo\r\tbar",
            "function foo() {\n    foo\n        barconsole.log(a);\n}",
        ),
        (
            p(2, 3),
            "foo\r\tbar",
            "function foo() {\n  foo\n    bar  console.log(a);\n}",
        ),
        (
            p(2, 5),
            "foo\r\tbar\nfoo",
            "function foo() {\n    foo\n        bar\n    fooconsole.log(a);\n}",
        ),
        // Indentation issue with choice elements that span multiple lines
        // #46266: the choice's text isn't re-indented.
        (
            p(2, 5),
            "a\nb${1|foo,\nbar|}",
            "function foo() {\n    a\n    bfooconsole.log(a);\n}",
        ),
    ];
    for (position, input, expected) in cases {
        ed.cancel(false);
        ed.set_value(original);
        ed.select(&[*position]);
        ed.insert(input);
        assert_eq!(ed.text(), *expected, "{input:?} at {position:?}");
    }
}

#[gpui::test]
fn adjust_selection_overwrite_before_after(cx: &mut TestAppContext) {
    // Monaco calls `SnippetSession.adjustSelection` directly; here the
    // adjusted range is replaced by a snippet instead.
    let mut ed = session_setup(cx);
    let original = "function foo() {\n    console.log(a);\n}";
    let cases: &[(u32, u32, &str)] = &[
        (1, 0, "Xunction foo() {"),
        (1111, 0, "Xunction foo() {"),
        (0, 10, "fXo() {"),
        (0, 10111, "fX"),
    ];
    for &(overwrite_before, overwrite_after, expected) in cases {
        ed.set_value(original);
        ed.select(&[p(1, 2)]);
        ed.insert_with(
            "X",
            SnippetInsertOptions {
                overwrite_before,
                overwrite_after,
                ..Default::default()
            },
        );
        assert_eq!(ed.line(1), expected);
    }
}

#[gpui::test]
fn text_edits_and_selection(cx: &mut TestAppContext) {
    let mut ed = session_setup(cx);
    ed.insert("foo${1:bar}foo$0");
    assert_eq!(
        ed.text(),
        "foobarfoofunction foo() {\n    foobarfooconsole.log(a);\n}"
    );
    ed.assert_selections(&[s(1, 4, 1, 7), s(2, 8, 2, 11)]);
    ed.next();
    ed.assert_selections(&[p(1, 10), p(2, 14)]);
}

#[gpui::test]
fn text_edit_with_reversed_selection(cx: &mut TestAppContext) {
    let mut ed = session_setup(cx);
    ed.select(&[p(2, 5), p(1, 1)]);
    ed.insert("${1:bar}$0");
    assert_eq!(ed.text(), "barfunction foo() {\n    barconsole.log(a);\n}");
    ed.assert_selections(&[s(2, 5, 2, 8), s(1, 1, 1, 4)]);
}

#[gpui::test]
fn repeated_tabstops(cx: &mut TestAppContext) {
    let mut ed = session_setup(cx);
    ed.insert("${1:abc}foo${1:abc}$0");
    ed.assert_selections(&[
        s(1, 1, 1, 4),
        s(1, 7, 1, 10),
        s(2, 5, 2, 8),
        s(2, 11, 2, 14),
    ]);
    ed.next();
    ed.assert_selections(&[p(1, 10), p(2, 14)]);
}

#[gpui::test]
fn just_text(cx: &mut TestAppContext) {
    let mut ed = session_setup(cx);
    ed.insert("foobar");
    assert_eq!(
        ed.text(),
        "foobarfunction foo() {\n    foobarconsole.log(a);\n}"
    );
    ed.assert_selections(&[p(1, 7), p(2, 11)]);
}

#[gpui::test]
fn selections_and_new_text_with_newlines(cx: &mut TestAppContext) {
    let mut ed = session_setup(cx);
    ed.insert("foo\n\t${1:bar}\n$0");
    assert_eq!(
        ed.text(),
        "foo\n    bar\nfunction foo() {\n    foo\n        bar\n    console.log(a);\n}"
    );
    ed.assert_selections(&[s(2, 5, 2, 8), s(5, 9, 5, 12)]);
    ed.next();
    ed.assert_selections(&[p(3, 1), p(6, 5)]);
}

#[gpui::test]
fn newline_no_whitespace_adjust(cx: &mut TestAppContext) {
    let mut ed = session_setup(cx);
    ed.select(&[p(2, 5)]);
    ed.insert_with(
        "abc\n    foo\n        bar\n$0",
        SnippetInsertOptions {
            adjust_whitespace: false,
            ..Default::default()
        },
    );
    assert_eq!(
        ed.text(),
        "function foo() {\n    abc\n    foo\n        bar\nconsole.log(a);\n}"
    );
}

#[gpui::test]
fn selections_next_prev(cx: &mut TestAppContext) {
    let mut ed = session_setup(cx);
    ed.insert("f$1oo${2:bar}foo$0");
    ed.assert_selections(&[p(1, 2), p(2, 6)]);
    ed.next();
    ed.assert_selections(&[s(1, 4, 1, 7), s(2, 8, 2, 11)]);
    ed.prev();
    ed.assert_selections(&[p(1, 2), p(2, 6)]);
    ed.next();
    ed.assert_selections(&[s(1, 4, 1, 7), s(2, 8, 2, 11)]);
    ed.next();
    ed.assert_selections(&[p(1, 10), p(2, 14)]);
}

#[gpui::test]
fn selections_and_typing(cx: &mut TestAppContext) {
    let mut ed = session_setup(cx);
    ed.insert("f${1:oo}_$2_$0");
    ed.type_text("X");
    ed.next();
    ed.type_text("bar");
    // Back to ${1:oo}, which is now just 'X'.
    ed.prev();
    ed.assert_selections(&[s(1, 2, 1, 3), s(2, 6, 2, 7)]);
    // Forward to $2, which is now 'bar'.
    ed.next();
    ed.assert_selections(&[s(1, 4, 1, 7), s(2, 8, 2, 11)]);
    ed.next();
    assert_eq!(
        ed.text(),
        "fX_bar_function foo() {\n    fX_bar_console.log(a);\n}"
    );
    ed.assert_selections(&[p(1, 8), p(2, 12)]);
}

#[gpui::test]
fn insert_shorter_snippet_into_non_empty_selection(cx: &mut TestAppContext) {
    let mut ed = session_setup(cx);
    ed.set_value("foo_bar_foo");
    ed.select(&[s(1, 1, 1, 4), s(1, 9, 1, 12)]);
    ed.insert("x$0");
    assert_eq!(ed.text(), "x_bar_x");
    ed.assert_selections(&[p(1, 2), p(1, 8)]);
}

#[gpui::test]
fn insert_longer_snippet_into_non_empty_selection(cx: &mut TestAppContext) {
    let mut ed = session_setup(cx);
    ed.set_value("foo_bar_foo");
    ed.select(&[s(1, 1, 1, 4), s(1, 9, 1, 12)]);
    ed.insert("LONGER$0");
    assert_eq!(ed.text(), "LONGER_bar_LONGER");
    ed.assert_selections(&[p(1, 7), p(1, 18)]);
}

#[gpui::test]
fn dont_grow_final_tabstop(cx: &mut TestAppContext) {
    // Monaco then types at $0 and moves back with the session; the
    // controller leaves snippet mode at $0, so that part isn't ported.
    let mut ed = session_setup(cx);
    ed.set_value("foo_zzz_foo");
    ed.select(&[s(1, 5, 1, 8)]);
    ed.insert("$1bar$0");
    ed.assert_selections(&[p(1, 5)]);
    ed.type_text("foo-");
    ed.next();
    assert_eq!(ed.text(), "foo_foo-bar_foo");
    ed.assert_selections(&[p(1, 12)]);
    ed.type_text("XXX");
    assert_eq!(ed.text(), "foo_foo-barXXX_foo");
}

#[gpui::test]
fn dont_merge_touching_tabstops_1(cx: &mut TestAppContext) {
    // Monaco moves to $0 and back with the session; through the controller
    // this stops one tab stop short of $0.
    let mut ed = session_setup(cx);
    ed.insert("$1$2$3$0");
    ed.assert_selections(&[p(1, 1), p(2, 5)]);
    ed.next();
    ed.assert_selections(&[p(1, 1), p(2, 5)]);
    ed.next();
    ed.assert_selections(&[p(1, 1), p(2, 5)]);
    ed.prev();
    ed.prev();
    ed.assert_selections(&[p(1, 1), p(2, 5)]);
    ed.type_text("111");
    ed.next();
    ed.type_text("222");
    ed.next();
    ed.type_text("333");
    ed.prev();
    ed.assert_selections(&[s(1, 4, 1, 7), s(2, 8, 2, 11)]);
    ed.prev();
    ed.assert_selections(&[s(1, 1, 1, 4), s(2, 5, 2, 8)]);
    ed.next();
    ed.next();
    ed.assert_selections(&[s(1, 7, 1, 10), s(2, 11, 2, 14)]);
    ed.next();
    assert_eq!(
        ed.text(),
        "111222333function foo() {\n    111222333console.log(a);\n}"
    );
    ed.assert_selections(&[p(1, 10), p(2, 14)]);
}

#[gpui::test]
fn dont_merge_touching_tabstops_2(cx: &mut TestAppContext) {
    let mut ed = session_setup(cx);
    ed.insert("$1$2$3$0");
    ed.assert_selections(&[p(1, 1), p(2, 5)]);
    ed.type_text("111");
    ed.next();
    ed.assert_selections(&[p(1, 4), p(2, 8)]);
    ed.type_text("222");
    ed.next();
    ed.assert_selections(&[p(1, 7), p(2, 11)]);
    ed.type_text("333");
    ed.next();
    // At the last placeholder: the controller leaves snippet mode.
    assert!(!ed.in_snippet());
}

#[gpui::test]
fn gracefully_move_over_final_tabstop(cx: &mut TestAppContext) {
    let mut ed = session_setup(cx);
    ed.insert("${1}bar$0");
    ed.assert_state(true, false, true);
    ed.assert_selections(&[p(1, 1), p(2, 5)]);
    ed.next();
    ed.assert_state(false, false, false);
    ed.assert_selections(&[p(1, 4), p(2, 8)]);
    ed.next();
    ed.assert_selections(&[p(1, 4), p(2, 8)]);
}

#[gpui::test]
fn overwriting_nested_placeholder(cx: &mut TestAppContext) {
    let mut ed = session_setup(cx);
    ed.insert("log(${1:\"$2\"});$0");
    ed.assert_selections(&[s(1, 5, 1, 7), s(2, 9, 2, 11)]);
    ed.type_text("XXX");
    assert_eq!(
        ed.text(),
        "log(XXX);function foo() {\n    log(XXX);console.log(a);\n}"
    );
    ed.next();
    assert!(ed.in_snippet());
    assert!(ed.state().2, "not at the last placeholder");
    ed.next();
    assert!(!ed.in_snippet());
    ed.assert_selections(&[p(1, 10), p(2, 14)]);
}

#[gpui::test]
fn selections_and_snippet_ranges(cx: &mut TestAppContext) {
    // Monaco asks `isSelectionWithinPlaceholders`; the controller leaves
    // snippet mode when it's false.
    let mut ed = session_setup(cx);
    let original = "function foo() {\n    console.log(a);\n}";
    let template = "${1:foo}farboo${2:bar}$0";
    let bad: &[&[Selection]] = &[
        &[p(1, 1)],
        // In the snippet, outside a placeholder.
        &[p(1, 6), p(2, 10)],
        &[p(1, 6), p(2, 10), p(1, 1)],
        &[p(1, 6), p(2, 10), s(2, 20, 2, 21)],
    ];
    for selections in bad {
        ed.cancel(false);
        ed.set_value(original);
        ed.select(&[p(1, 1), p(2, 5)]);
        ed.insert(template);
        assert_eq!(
            ed.text(),
            "foofarboobarfunction foo() {\n    foofarboobarconsole.log(a);\n}"
        );
        ed.assert_selections(&[s(1, 1, 1, 4), s(2, 5, 2, 8)]);
        assert!(ed.in_snippet());
        ed.select(selections);
        assert!(!ed.in_snippet(), "{selections:?}");
    }

    ed.set_value(original);
    ed.select(&[p(1, 1), p(2, 5)]);
    ed.insert(template);
    ed.next();
    assert!(ed.in_snippet());
    ed.assert_selections(&[s(1, 10, 1, 13), s(2, 14, 2, 17)]);
    ed.next();
    assert!(!ed.in_snippet());
    ed.assert_selections(&[p(1, 13), p(2, 17)]);
}

#[gpui::test]
fn nested_sessions(cx: &mut TestAppContext) {
    // Monaco runs two independent sessions; through the controller the
    // second is nested into the first.
    let mut ed = session_setup(cx);
    ed.set_value("");
    ed.select(&[p(1, 1)]);
    ed.insert("foo${2:bar}foo$0");
    assert_eq!(ed.text(), "foobarfoo");
    ed.assert_selections(&[s(1, 4, 1, 7)]);

    ed.insert("ba${1:zzzz}$0");
    assert_eq!(ed.text(), "foobazzzzfoo");
    ed.assert_selections(&[s(1, 6, 1, 10)]);

    ed.next();
    ed.assert_selections(&[p(1, 10)]);
    assert!(ed.in_snippet());

    ed.next();
    assert!(!ed.in_snippet());
    ed.assert_selections(&[p(1, 13)]);
}

#[gpui::test]
fn typing_at_final_tabstop(cx: &mut TestAppContext) {
    let mut ed = session_setup(cx);
    ed.insert("farboo$0");
    assert!(!ed.in_snippet());
    ed.type_text("XXX");
    assert!(!ed.in_snippet());
}

#[gpui::test]
fn typing_at_beginning(cx: &mut TestAppContext) {
    let mut ed = session_setup(cx);
    ed.select(&[p(1, 2)]);
    ed.insert("farboo$0");
    ed.select(&[p(1, 2)]);
    assert!(!ed.in_snippet());
    ed.type_text("XXX");
    assert_eq!(ed.line(1), "fXXXfarboounction foo() {");
    assert!(!ed.in_snippet());
}

#[gpui::test]
fn typing_with_nested_placeholder(cx: &mut TestAppContext) {
    let mut ed = session_setup(cx);
    ed.select(&[p(1, 1)]);
    ed.insert("This ${1:is ${2:nested}}.$0");
    ed.assert_selections(&[s(1, 6, 1, 15)]);
    ed.next();
    ed.assert_selections(&[s(1, 9, 1, 15)]);
    ed.cut();
    ed.assert_selections(&[p(1, 9)]);
    ed.type_text("XXX");
    ed.prev();
    ed.assert_selections(&[s(1, 6, 1, 12)]);
}

#[gpui::test]
fn snippet_with_variables(cx: &mut TestAppContext) {
    let mut ed = session_setup(cx);
    ed.insert("@line=$TM_LINE_NUMBER$0");
    assert_eq!(
        ed.text(),
        "@line=1function foo() {\n    @line=2console.log(a);\n}"
    );
    ed.assert_selections(&[p(1, 8), p(2, 12)]);
}

#[gpui::test]
fn merge(cx: &mut TestAppContext) {
    let mut ed = session_setup(cx);
    ed.select(&[p(1, 1)]);
    ed.insert("This ${1:is ${2:nested}}.$0");
    ed.next();
    ed.assert_selections(&[s(1, 9, 1, 15)]);

    ed.insert("really ${1:nested}$0");
    ed.assert_selections(&[s(1, 16, 1, 22)]);

    ed.next();
    ed.assert_selections(&[p(1, 22)]);
    assert!(ed.state().2, "not at the last placeholder");

    // Monaco goes on to the outer $0 and back here; the controller would
    // leave snippet mode at $0.
    ed.type_text("AAA");

    // Back to `really ${1:nested}`.
    ed.prev();
    ed.assert_selections(&[s(1, 16, 1, 22)]);

    // Back to `${1:is ...}`, which grew.
    ed.prev();
    ed.assert_selections(&[s(1, 6, 1, 25)]);
}

#[gpui::test]
fn next_does_not_panic_when_placeholder_decorations_are_missing(cx: &mut TestAppContext) {
    let mut ed = session_setup(cx);
    ed.set_value("");
    ed.select(&[p(1, 1)]);
    ed.insert("${1:foo}$0");

    let owner = ed.editor.entity_id();
    let model = ed.editor.read_with(ed.cx, |e, _| e.model().clone());
    model.update(ed.cx, |m, cx| {
        let ids: Vec<_> = m
            .all_decorations(Some(owner))
            .into_iter()
            .filter(|d| d.owner == Some(owner))
            .map(|d| d.id)
            .collect();
        assert!(!ids.is_empty());
        m.delta_decorations(Some(owner), &ids, Vec::new(), cx);
    });
    ed.next();
}

#[gpui::test]
fn deep_merge_does_not_produce_phantom_cursors(cx: &mut TestAppContext) {
    // #279349
    let mut ed = session_setup(cx);
    ed.set_value("");
    ed.select(&[p(1, 1)]);
    ed.insert(r"\sin\left(${1:x}\right) ");
    for i in 0..25 {
        ed.insert(r"\sin\left(${1:x}\right) ");
        assert_eq!(
            ed.selections().len(),
            1,
            "selection count after {} merges",
            i + 1
        );
    }
}

#[gpui::test]
fn merge_preserves_mirrors_in_nested_snippet(cx: &mut TestAppContext) {
    let mut ed = session_setup(cx);
    ed.set_value("");
    ed.select(&[p(1, 1)]);
    ed.insert("(${1:x})");
    ed.insert("${1:y}-${1}");
    assert_eq!(ed.text(), "(y-y)");
    assert_eq!(ed.selections().len(), 2);
}

#[gpui::test]
fn merge_when_placeholder_occurrences_collapse_to_same_position(cx: &mut TestAppContext) {
    let mut ed = session_setup(cx);
    ed.set_value("");
    ed.select(&[p(1, 1)]);
    ed.insert("$1$1$0");
    ed.insert("${1:nested}$0");
    assert_eq!(ed.text(), "nested");
}

#[gpui::test]
fn transform(cx: &mut TestAppContext) {
    let mut ed = session_setup(cx);
    ed.set_value("");
    ed.select(&[p(1, 1)]);
    ed.insert("${1/foo/bar/}$0");
    ed.assert_selections(&[p(1, 1)]);
    ed.type_text("foo");
    ed.next();
    assert_eq!(ed.text(), "bar");
    assert!(!ed.in_snippet());
    ed.assert_selections(&[p(1, 4)]);
}

#[gpui::test]
fn multi_placeholder_same_index_one_transform(cx: &mut TestAppContext) {
    let mut ed = session_setup(cx);
    ed.set_value("");
    ed.select(&[p(1, 1)]);
    ed.insert("$1 baz ${1/foo/bar/}$0");
    ed.assert_selections(&[p(1, 1), p(1, 6)]);
    ed.type_text("foo");
    ed.next();
    assert_eq!(ed.text(), "foo baz bar");
    assert!(!ed.in_snippet());
    ed.assert_selections(&[p(1, 12)]);
}

const TRANSFORM_EXAMPLE: &str = "${1:name} : ${2:type}${3/\\s:=(.*)/${1:+ :=}${1}/};\n$0";

#[gpui::test]
fn transform_example(cx: &mut TestAppContext) {
    let mut ed = session_setup(cx);
    ed.set_value("");
    ed.select(&[p(1, 1)]);
    ed.insert(TRANSFORM_EXAMPLE);
    ed.assert_selections(&[s(1, 1, 1, 5)]);
    ed.type_text("clk");
    ed.next();
    ed.assert_selections(&[s(1, 7, 1, 11)]);
    ed.type_text("std_logic");
    ed.next();
    ed.assert_selections(&[p(1, 16)]);
    ed.next();
    assert_eq!(ed.text(), "clk : std_logic;\n");
    assert!(!ed.in_snippet());
    ed.assert_selections(&[p(2, 1)]);
}

#[gpui::test]
fn transform_with_indent(cx: &mut TestAppContext) {
    let mut ed = session_setup(cx);
    let snippet = [
        "private readonly ${1} = new Emitter<$2>();",
        "readonly ${1/^_(.*)/$1/}: Event<$2> = this.$1.event;",
        "$0",
    ]
    .join("\n");
    let expected = [
        "{",
        "\tprivate readonly _prop = new Emitter<string>();",
        "\treadonly prop: Event<string> = this._prop.event;",
        "\t",
        "}",
    ]
    .join("\n");
    ed.set_value("{\n\t\n}");
    ed.set_model_options(false, 4, 4);
    ed.select(&[p(2, 2)]);
    ed.insert(&snippet);
    ed.assert_selections(&[p(2, 19), p(3, 11), p(3, 28)]);
    ed.type_text("_prop");
    ed.next();
    ed.assert_selections(&[p(2, 39), p(3, 23)]);
    ed.type_text("string");
    ed.next();
    assert_eq!(ed.text(), expected);
    assert!(!ed.in_snippet());
    ed.assert_selections(&[p(4, 2)]);
}

#[gpui::test]
fn transform_example_hit_if(cx: &mut TestAppContext) {
    let mut ed = session_setup(cx);
    ed.set_value("");
    ed.select(&[p(1, 1)]);
    ed.insert(TRANSFORM_EXAMPLE);
    ed.assert_selections(&[s(1, 1, 1, 5)]);
    ed.type_text("clk");
    ed.next();
    ed.assert_selections(&[s(1, 7, 1, 11)]);
    ed.type_text("std_logic");
    ed.next();
    ed.assert_selections(&[p(1, 16)]);
    ed.type_text(" := '1'");
    ed.next();
    assert_eq!(ed.text(), "clk : std_logic := '1';\n");
    assert!(!ed.in_snippet());
    ed.assert_selections(&[p(2, 1)]);
}

#[gpui::test]
fn transform_adjacent_to_previous_placeholder_96545(cx: &mut TestAppContext) {
    let mut ed = session_setup(cx);
    ed.set_value("");
    ed.select(&[p(1, 1)]);
    ed.insert("${1:{}${2:fff}${1/{/}/}");
    ed.assert_selections(&[s(1, 1, 1, 2), s(1, 5, 1, 6)]);
    ed.next();
    assert_eq!(ed.text(), "{fff}");
    ed.assert_selections(&[s(1, 2, 1, 5)]);
    ed.type_text("ggg");
    ed.next();
    assert_eq!(ed.text(), "{ggg}");
    assert!(!ed.in_snippet());
    ed.assert_selections(&[p(1, 6)]);
}

#[gpui::test]
fn tab_stop_selection_96545(cx: &mut TestAppContext) {
    let mut ed = session_setup(cx);
    ed.set_value("");
    ed.select(&[p(1, 1)]);
    ed.insert(r"${1:{}${2:fff}${1/[\{]/}/}$0");
    assert_eq!(ed.text(), "{fff{");
    ed.assert_selections(&[s(1, 1, 1, 2), s(1, 5, 1, 6)]);
    ed.next();
    ed.assert_selections(&[s(1, 2, 1, 5)]);
}

#[gpui::test]
fn placeholder_index_after_snippets_ending_with_placeholder_30769(cx: &mut TestAppContext) {
    let mut ed = session_setup(cx);
    ed.set_value("");
    ed.select(&[p(1, 1)]);
    ed.insert("test ${1:replaceme}");
    ed.type_text("1");
    ed.keys("enter");
    assert_eq!(ed.text(), "test 1\n");

    ed.insert("test ${1:replaceme}");
    ed.type_text("2");
    ed.keys("enter");
    assert_eq!(ed.text(), "test 1\ntest 2\n");

    ed.insert("test ${1:replaceme}");
    ed.type_text("3");
    ed.keys("enter");
    assert_eq!(ed.text(), "test 1\ntest 2\ntest 3\n");

    ed.insert("test ${1:replaceme}");
    ed.type_text("4");
    ed.keys("enter");
    assert_eq!(ed.text(), "test 1\ntest 2\ntest 3\ntest 4\n");
}

#[gpui::test]
fn variable_text_isnt_whitespace_normalised_31124(cx: &mut TestAppContext) {
    let mut ed = session_setup(cx);
    ed.set_value("start\n\t\t-one\n\t\t-two\nend");
    ed.set_model_options(false, 4, 4);
    ed.select(&[s(2, 2, 3, 7)]);
    ed.insert("<div>\n\t$TM_SELECTED_TEXT\n</div>$0");
    assert_eq!(
        ed.text(),
        "start\n\t<div>\n\t\t\t-one\n\t\t\t-two\n\t</div>\nend"
    );

    ed.set_value("start\n\t\t-one\n\t-two\nend");
    ed.select(&[s(2, 2, 3, 7)]);
    ed.insert("<div>\n\t$TM_SELECTED_TEXT\n</div>$0");
    assert_eq!(
        ed.text(),
        "start\n\t<div>\n\t\t\t-one\n\t\t-two\n\t</div>\nend"
    );
}

#[gpui::test]
fn selecting_left_to_right_and_choosing_item_31199(cx: &mut TestAppContext) {
    // The `adjustSelection` assertions of this test aren't reachable; the
    // insertion is.
    let mut ed = session_setup(cx);
    ed.set_value("console.log");
    ed.select(&[s(1, 9, 1, 12)]);
    ed.insert_with("far", before(3));
    assert_eq!(ed.text(), "console.far");
}

#[gpui::test]
fn tabs_dont_get_replaced_with_spaces_in_transformations_103818(cx: &mut TestAppContext) {
    let mut ed = session_setup(cx);
    ed.set_value("\n{\n  \n}");
    ed.set_model_options(true, 4, 2);
    ed.select(&[p(1, 1), p(3, 6)]);
    ed.insert(
        &[
            "function animate () {",
            "\tvar ${1:a} = 12;",
            "\tconsole.log(${1/(.*)/\n\t\t$1\n\t/})",
            "}",
        ]
        .join("\n"),
    );
    assert_eq!(
        ed.text(),
        [
            "function animate () {",
            "  var a = 12;",
            "  console.log(a)",
            "}",
            "{",
            "  function animate () {",
            "    var a = 12;",
            "    console.log(a)",
            "  }",
            "}",
        ]
        .join("\n")
    );
    ed.type_text("bbb");
    ed.next();
    assert_eq!(
        ed.text(),
        [
            "function animate () {",
            "  var bbb = 12;",
            "  console.log(",
            "    bbb",
            "  )",
            "}",
            "{",
            "  function animate () {",
            "    var bbb = 12;",
            "    console.log(",
            "      bbb",
            "    )",
            "  }",
            "}",
        ]
        .join("\n")
    );
}

// Skipped: the `createEditsAndSnippetsFromEdits` suite (empty, basic, with
// $SELECTION variable, $TM_SELECTED_TEXT/$TM_LINE_NUMBER per edit,
// per-edit resolution, $CURSOR_NUMBER order, cross-edit backfill) tests
// `SnippetSession.createEditsAndSnippetsFromEdits` / `apply`, which we
// don't have.

// ----- snippetController2.test.ts -------------------------------------------

#[gpui::test]
fn creation(cx: &mut TestAppContext) {
    let mut ed = controller_setup(cx);
    ed.assert_state(false, false, false);
}

#[gpui::test]
fn insert_abort(cx: &mut TestAppContext) {
    let mut ed = controller_setup(cx);
    ed.insert("foo${1:bar}foo$0");
    ed.assert_state(true, false, true);
    ed.assert_selections(&[s(1, 4, 1, 7), s(2, 8, 2, 11)]);

    ed.cancel(false);
    ed.assert_state(false, false, false);
    ed.assert_selections(&[s(1, 4, 1, 7), s(2, 8, 2, 11)]);
}

#[gpui::test]
fn insert_tab_tab_done(cx: &mut TestAppContext) {
    let mut ed = controller_setup(cx);
    ed.insert("${1:one}${2:two}$0");
    ed.assert_state(true, false, true);
    ed.next();
    ed.assert_state(true, true, true);
    ed.next();
    ed.assert_state(false, false, false);
    ed.type_text("\t");
    ed.assert_state(false, false, false);
}

#[gpui::test]
fn insert_with_keys_tab_shift_tab_escape(cx: &mut TestAppContext) {
    // `insert, insert -> tab, tab, done` through the keybindings.
    let mut ed = controller_setup(cx);
    ed.insert("${1:one}${2:two}$0");
    ed.keys("tab");
    ed.assert_state(true, true, true);
    ed.assert_selections(&[s(1, 4, 1, 7), s(2, 8, 2, 11)]);
    ed.keys("shift-tab");
    ed.assert_state(true, false, true);
    ed.assert_selections(&[s(1, 1, 1, 4), s(2, 5, 2, 8)]);
    ed.keys("escape");
    ed.assert_state(false, false, false);
    ed.assert_selections(&[s(1, 1, 1, 4)]);
}

#[gpui::test]
fn insert_cursor_moves_out_left_right(cx: &mut TestAppContext) {
    let mut ed = controller_setup(cx);
    ed.insert("foo${1:bar}foo$0");
    ed.assert_state(true, false, true);
    ed.assert_selections(&[s(1, 4, 1, 7), s(2, 8, 2, 11)]);
    ed.select(&[p(1, 12), p(2, 16)]);
    ed.assert_state(false, false, false);
}

#[gpui::test]
fn insert_cursor_moves_out_up_down(cx: &mut TestAppContext) {
    let mut ed = controller_setup(cx);
    ed.insert("foo${1:bar}foo$0");
    ed.assert_state(true, false, true);
    ed.assert_selections(&[s(1, 4, 1, 7), s(2, 8, 2, 11)]);
    ed.select(&[s(2, 4, 2, 7), s(3, 8, 3, 11)]);
    ed.assert_state(false, false, false);
}

#[gpui::test]
fn insert_cursors_collapse(cx: &mut TestAppContext) {
    let mut ed = controller_setup(cx);
    ed.insert("foo${1:bar}foo$0");
    assert!(ed.in_snippet());
    ed.assert_selections(&[s(1, 4, 1, 7), s(2, 8, 2, 11)]);
    ed.select(&[s(1, 4, 1, 7)]);
    ed.assert_state(false, false, false);
}

#[gpui::test]
fn insert_plain_text_no_snippet_mode(cx: &mut TestAppContext) {
    let mut ed = controller_setup(cx);
    ed.insert("foobar");
    ed.assert_state(false, false, false);
    ed.assert_selections(&[p(1, 7), p(2, 11)]);
}

#[gpui::test]
fn insert_delete_snippet_text(cx: &mut TestAppContext) {
    let mut ed = controller_setup(cx);
    ed.insert("${1:foobar}$0");
    ed.assert_state(true, false, true);
    ed.assert_selections(&[s(1, 1, 1, 7), s(2, 5, 2, 11)]);

    ed.cut();
    ed.assert_state(true, false, true);
    ed.assert_selections(&[p(1, 1), p(2, 5)]);

    ed.type_text("abc");
    ed.assert_state(true, false, true);

    ed.next();
    ed.assert_state(false, false, false);

    ed.keys("tab");
    ed.assert_state(false, false, false);
}

#[gpui::test]
fn insert_nested_trivial_snippet(cx: &mut TestAppContext) {
    let mut ed = controller_setup(cx);
    ed.insert("${1:foo}bar$0");
    ed.assert_state(true, false, true);
    ed.assert_selections(&[s(1, 1, 1, 4), s(2, 5, 2, 8)]);

    ed.insert("FOO$0");
    ed.assert_selections(&[p(1, 4), p(2, 8)]);
    ed.assert_state(true, false, true);

    ed.next();
    ed.assert_selections(&[p(1, 7), p(2, 11)]);
    ed.assert_state(false, false, false);
}

#[gpui::test]
fn insert_nested_snippet(cx: &mut TestAppContext) {
    let mut ed = controller_setup(cx);
    ed.insert("${1:foobar}$0");
    ed.assert_state(true, false, true);
    ed.assert_selections(&[s(1, 1, 1, 7), s(2, 5, 2, 11)]);

    ed.insert("far$1boo$0");
    ed.assert_selections(&[p(1, 4), p(2, 8)]);
    ed.assert_state(true, false, true);

    ed.next();
    ed.assert_selections(&[p(1, 7), p(2, 11)]);
    ed.assert_state(true, true, true);

    ed.next();
    ed.assert_selections(&[p(1, 7), p(2, 11)]);
    ed.assert_state(false, false, false);
}

#[gpui::test]
fn insert_nested_plain_text(cx: &mut TestAppContext) {
    let mut ed = controller_setup(cx);
    ed.insert("${1:foobar}$0");
    ed.assert_state(true, false, true);
    ed.assert_selections(&[s(1, 1, 1, 7), s(2, 5, 2, 11)]);

    ed.insert("farboo");
    ed.assert_selections(&[p(1, 7), p(2, 11)]);
    ed.assert_state(true, false, true);

    ed.next();
    ed.assert_selections(&[p(1, 7), p(2, 11)]);
    ed.assert_state(false, false, false);
}

#[gpui::test]
fn nested_snippets_without_final_placeholder_27898(cx: &mut TestAppContext) {
    let mut ed = controller_setup(cx);
    ed.insert("for(const ${1:element} of ${2:array}) {$0}");
    ed.assert_state(true, false, true);
    ed.assert_selections(&[s(1, 11, 1, 18), s(2, 15, 2, 22)]);

    ed.next();
    ed.assert_state(true, true, true);
    ed.assert_selections(&[s(1, 22, 1, 27), s(2, 26, 2, 31)]);

    ed.insert("document");
    ed.assert_state(true, true, true);
    ed.assert_selections(&[p(1, 30), p(2, 34)]);

    ed.next();
    ed.assert_state(false, false, false);
}

#[gpui::test]
fn recursive_snippets_and_tab_shift_tab_27543(cx: &mut TestAppContext) {
    let mut ed = controller_setup(cx);
    ed.insert("1_calize(${1:nl}, '${2:value}')$0");
    ed.assert_state(true, false, true);
    ed.assert_selections(&[s(1, 10, 1, 12), s(2, 14, 2, 16)]);

    ed.insert("2_calize(${1:nl}, '${2:value}')$0");
    ed.assert_selections(&[s(1, 19, 1, 21), s(2, 23, 2, 25)]);

    ed.next(); // inner `value`
    ed.assert_selections(&[s(1, 24, 1, 29), s(2, 28, 2, 33)]);

    ed.next(); // inner `$0`
    ed.assert_selections(&[p(1, 31), p(2, 35)]);

    ed.next(); // outer `value`
    ed.assert_selections(&[s(1, 34, 1, 39), s(2, 38, 2, 43)]);

    ed.prev(); // inner `$0`
    ed.assert_selections(&[p(1, 31), p(2, 35)]);
}

#[gpui::test]
fn tabstop_selecting_content_of_previously_entered_variable_23728(cx: &mut TestAppContext) {
    let mut ed = controller_setup(cx);
    ed.set_value("");
    ed.select(&[p(1, 1)]);
    ed.insert("import ${2:${1:module}} from '${1:module}'$0");
    ed.assert_state(true, false, true);
    ed.assert_selections(&[s(1, 8, 1, 14), s(1, 21, 1, 27)]);

    ed.insert("foo");
    ed.assert_selections(&[p(1, 11), p(1, 21)]);

    ed.next(); // ${2:...}
    ed.assert_selections(&[s(1, 8, 1, 11)]);
}

#[gpui::test]
fn html_snippets_combine_32211(cx: &mut TestAppContext) {
    let mut ed = controller_setup(cx);
    ed.set_value("");
    ed.set_model_options(false, 4, 4);
    ed.select(&[p(1, 1)]);
    ed.insert(concat!(
        "\n",
        "\t\t\t<!DOCTYPE html>\n",
        "\t\t\t<html lang=\"en\">\n",
        "\t\t\t<head>\n",
        "\t\t\t\t<meta charset=\"UTF-8\">\n",
        "\t\t\t\t<meta name=\"viewport\" content=\"width=${2:device-width}, initial-scale=${3:1.0}\">\n",
        "\t\t\t\t<meta http-equiv=\"X-UA-Compatible\" content=\"${5:ie=edge}\">\n",
        "\t\t\t\t<title>${7:Document}</title>\n",
        "\t\t\t</head>\n",
        "\t\t\t<body>\n",
        "\t\t\t\t${8}\n",
        "\t\t\t</body>\n",
        "\t\t\t</html>\n",
        "\t\t",
    ));
    ed.next();
    ed.next();
    ed.next();
    ed.next();
    ed.assert_selections(&[p(11, 5)]);

    ed.insert("<input type=\"${2:text}\">");
    ed.assert_selections(&[s(11, 18, 11, 22)]);
}

#[gpui::test]
fn problems_with_nested_snippet_insertion_39594(cx: &mut TestAppContext) {
    let mut ed = controller_setup(cx);
    ed.set_value("");
    ed.select(&[p(1, 1)]);
    ed.insert("$1 = ConvertTo-Json $1");
    ed.assert_selections(&[p(1, 1), p(1, 19)]);

    // $1 has two occurrences and only one selection is left.
    ed.select(&[p(1, 19)]);
    ed.assert_state(false, false, false);
}

#[gpui::test]
fn problems_with_nested_snippet_insertion_39594_part2(cx: &mut TestAppContext) {
    // The selection-change-to-cancel logic isn't too aggressive.
    let mut ed = controller_setup(cx);
    ed.set_value("a-\naaa-");
    ed.select(&[p(2, 5), p(1, 3)]);
    ed.insert("log($1);$0");
    ed.assert_selections(&[p(2, 9), p(1, 7)]);
    ed.assert_state(true, false, true);
}

#[gpui::test]
fn nested_snippets_terminating_abruptly_42012(cx: &mut TestAppContext) {
    let mut ed = controller_setup(cx);
    ed.set_value("");
    ed.select(&[p(1, 1)]);
    ed.insert("var ${2:${1:name}} = ${1:name} + 1;${0}");
    ed.assert_selections(&[s(1, 5, 1, 9), s(1, 12, 1, 16)]);
    ed.assert_state(true, false, true);

    ed.next();
    ed.assert_state(true, true, true);
}

#[gpui::test]
fn placeholders_order_58267(cx: &mut TestAppContext) {
    let mut ed = controller_setup(cx);
    ed.set_value("");
    ed.select(&[p(1, 1)]);
    ed.insert(r"\pth{$1}$0");
    ed.assert_selections(&[p(1, 6)]);
    ed.assert_state(true, false, true);

    ed.insert(r"\itv{${1:left}}{${2:right}}{${3:left_value}}{${4:right_value}}$0");
    ed.assert_selections(&[s(1, 11, 1, 15)]);

    ed.next();
    ed.assert_selections(&[s(1, 17, 1, 22)]);
    ed.next();
    ed.assert_selections(&[s(1, 24, 1, 34)]);
    ed.next();
    ed.assert_selections(&[s(1, 36, 1, 47)]);
    ed.next();
    ed.assert_selections(&[p(1, 48)]);
    ed.next();
    ed.assert_selections(&[p(1, 49)]);
    ed.assert_state(false, false, false);
}

#[gpui::test]
fn must_tab_through_deleted_tab_stops_31619(cx: &mut TestAppContext) {
    let mut ed = controller_setup(cx);
    ed.set_value("");
    ed.select(&[p(1, 1)]);
    ed.insert("foo${1:a${2:bar}baz}end$0");
    ed.assert_selections(&[s(1, 4, 1, 11)]);

    ed.cut();
    ed.assert_selections(&[p(1, 4)]);

    ed.next();
    ed.assert_selections(&[p(1, 7)]);
    ed.assert_state(false, false, false);
}

const REGION: &str = ".REGION ${2:FUNCTION_NAME}\nCREATE.FUNCTION ${1:VOID} ${2:FUNCTION_NAME}(${3:})\n\t${4:}\nEND\n.ENDREGION$0";

#[gpui::test]
fn cancelling_snippet_mode_discards_added_cursors_68512_soft(cx: &mut TestAppContext) {
    let mut ed = controller_setup(cx);
    ed.set_value("");
    ed.select(&[p(1, 1)]);
    ed.insert(REGION);
    ed.assert_selections(&[s(2, 17, 2, 21)]);

    ed.next();
    ed.assert_selections(&[s(1, 9, 1, 22), s(2, 22, 2, 35)]);
    ed.assert_state(true, true, true);

    ed.select(&[p(1, 22), p(2, 35)]);
    ed.assert_state(true, true, true);

    ed.select(&[p(2, 1), p(2, 36)]);
    ed.assert_state(false, false, false);
    ed.assert_selections(&[p(2, 1), p(2, 36)]);
}

#[gpui::test]
fn cancelling_snippet_mode_discards_added_cursors_68512_hard(cx: &mut TestAppContext) {
    let mut ed = controller_setup(cx);
    ed.set_value("");
    ed.select(&[p(1, 1)]);
    ed.insert(REGION);
    ed.assert_selections(&[s(2, 17, 2, 21)]);

    ed.next();
    ed.assert_selections(&[s(1, 9, 1, 22), s(2, 22, 2, 35)]);
    ed.assert_state(true, true, true);

    ed.select(&[p(1, 22), p(2, 35)]);
    ed.assert_state(true, true, true);

    ed.cancel(true);
    ed.assert_state(false, false, false);
    ed.assert_selections(&[p(1, 22)]);
}

#[gpui::test]
fn user_defined_snippet_tab_stops_ignored_72862(cx: &mut TestAppContext) {
    let mut ed = controller_setup(cx);
    ed.set_value("");
    ed.select(&[p(1, 1)]);
    ed.insert("export default $1");
    ed.assert_state(true, false, true);
}

#[gpui::test]
fn optional_tabstop_in_snippets_72358(cx: &mut TestAppContext) {
    let mut ed = controller_setup(cx);
    ed.set_value("");
    ed.select(&[p(1, 1)]);
    ed.insert("${1:prop: {$2\\},}\nmore$0");
    ed.assert_state(true, false, true);
    ed.assert_selections(&[s(1, 1, 1, 10)]);

    ed.cut();
    ed.assert_selections(&[p(1, 1)]);

    ed.next();
    ed.assert_selections(&[p(2, 5)]);
    ed.assert_state(false, false, false);
}

#[gpui::test]
fn confusing_trim_whitespace_edits_90135(cx: &mut TestAppContext) {
    let mut ed = controller_setup(cx);
    ed.set_value("");
    ed.select(&[p(1, 1)]);
    ed.keys("tab");
    ed.insert("\nfoo");
    ed.assert_selections(&[p(2, 8)]);
}

#[gpui::test]
fn insert_snippet_selections_in_wrong_positions_145727_1(cx: &mut TestAppContext) {
    let mut ed = controller_setup(cx);
    ed.set_value("");
    ed.select(&[p(1, 1)]);
    ed.keys("tab");
    ed.insert_with(
        "\naProperty: aClass<${2:boolean}> = new aClass<${2:boolean}>();\n",
        SnippetInsertOptions {
            adjust_whitespace: false,
            ..Default::default()
        },
    );
    ed.assert_selections(&[s(2, 19, 2, 26), s(2, 41, 2, 48)]);
}

#[gpui::test]
fn insert_snippet_selections_in_wrong_positions_145727_2(cx: &mut TestAppContext) {
    let mut ed = controller_setup(cx);
    ed.set_value("");
    ed.select(&[p(1, 1)]);
    ed.keys("tab");
    ed.insert("\naProperty: aClass<${2:boolean}> = new aClass<${2:boolean}>();\n");
    // Inserts `\n    aProperty...`.
    ed.assert_selections(&[s(2, 23, 2, 30), s(2, 45, 2, 52)]);
}

#[gpui::test]
fn leading_tab_replaced_by_spaces_101870(cx: &mut TestAppContext) {
    let mut ed = controller_setup(cx);
    ed.set_value("");
    ed.set_model_options(true, 4, 4);
    ed.select(&[p(1, 1)]);
    ed.insert("\tHello World\n\tNew Line");
    assert_eq!(ed.text(), "    Hello World\n    New Line");
}

#[gpui::test]
fn leading_tab_replaced_by_spaces_101870_part2(cx: &mut TestAppContext) {
    let mut ed = controller_setup(cx);
    ed.set_value("");
    ed.set_model_options(true, 4, 4);
    ed.select(&[p(1, 1)]);
    ed.insert("\tHello World\n\tNew Line\n${1:\tmore}");
    assert_eq!(ed.text(), "    Hello World\n    New Line\n    more");
}

// Skipped: `Snippet transformation does not work after inserting variable
// using intellisense, #112362` is `test.skip` in Monaco.
//
// Skipped: the controller `createEditsAndSnippetsFromEdits` suite (apply,
// tab, done; with special final tabstop; apply, tab, tab, done; nested into
// apply works; nested into insert abort "outer" snippet (1 and 2)), `EOL-
// Sequence (CRLF) shifts tab stop in isFileTemplate snippets #167386`,
// `"Surround With" code action snippets use incorrect indentation #169319`,
// `$TM_SELECTED_TEXT resolves per edit (multi-cursor wrap snippet)
// #206121` and `apply with multiple cursors threads $CURSOR_NUMBER per
// edit`: they need `SnippetController2.apply` with `ISnippetEdit`s.

#[gpui::test]
fn cursor_position_final_tabstop_with_user_snippets_163808(cx: &mut TestAppContext) {
    let mut ed = controller_setup(cx);
    ed.set_value("");
    ed.select(&[p(1, 1)]);
    ed.insert("<Element1 Attr1=\"foo\" $1>\n  <Element2 Attr1=\"$2\"/>\n$0\"\n</Element1>");
    ed.assert_selections(&[p(1, 23)]);

    ed.insert("Qualifier=\"$0\"");
    assert_eq!(
        ed.text(),
        "<Element1 Attr1=\"foo\" Qualifier=\"\">\n  <Element2 Attr1=\"\"/>\n\"\n</Element1>"
    );
    ed.assert_selections(&[p(1, 34)]);
}

#[gpui::test]
fn undo_restores_original_selection_after_insert_170041(cx: &mut TestAppContext) {
    // Monaco's test uses `apply` with an edit away from the cursor; here the
    // snippet replaces the selection instead.
    let mut ed = controller_setup(cx);
    ed.set_value("Some text and more text");
    ed.select(&[s(1, 6, 1, 10)]);
    ed.insert("${0:hi}");
    assert_eq!(ed.text(), "Some hi and more text");
    ed.undo();
    assert_eq!(ed.text(), "Some text and more text");
    ed.assert_selections(&[s(1, 6, 1, 10)]);
}

// ----- snippetController2.old.test.ts ---------------------------------------

#[gpui::test]
fn old_simple_accepted(cx: &mut TestAppContext) {
    let mut ed = old_setup(OLD_LINES, cx);
    ed.select(&[p(4, 2)]);
    ed.insert(OLD_TEMPLATE);
    assert_eq!(
        ed.line(4),
        "\tfor (var index; index < array.length; index++) {"
    );
    assert_eq!(ed.line(5), "\t\tvar element = array[index];");
    assert_eq!(ed.line(6), "\t\t");
    assert_eq!(ed.line(7), "\t}");

    ed.type_text("i");
    assert_eq!(ed.line(4), "\tfor (var i; i < array.length; i++) {");
    assert_eq!(ed.line(5), "\t\tvar element = array[i];");
    assert_eq!(ed.line(6), "\t\t");
    assert_eq!(ed.line(7), "\t}");

    ed.next();
    ed.type_text("arr");
    assert_eq!(ed.line(4), "\tfor (var i; i < arr.length; i++) {");
    assert_eq!(ed.line(5), "\t\tvar element = arr[i];");
    assert_eq!(ed.line(6), "\t\t");
    assert_eq!(ed.line(7), "\t}");

    ed.prev();
    ed.type_text("j");
    assert_eq!(ed.line(4), "\tfor (var j; j < arr.length; j++) {");
    assert_eq!(ed.line(5), "\t\tvar element = arr[j];");
    assert_eq!(ed.line(6), "\t\t");
    assert_eq!(ed.line(7), "\t}");

    ed.next();
    ed.next();
    ed.assert_selections(&[p(6, 3)]);
}

#[gpui::test]
fn old_simple_canceled(cx: &mut TestAppContext) {
    let mut ed = old_setup(OLD_LINES, cx);
    ed.select(&[p(4, 2)]);
    ed.insert(OLD_TEMPLATE);
    assert_eq!(
        ed.line(4),
        "\tfor (var index; index < array.length; index++) {"
    );
    assert_eq!(ed.line(5), "\t\tvar element = array[index];");
    assert_eq!(ed.line(6), "\t\t");
    assert_eq!(ed.line(7), "\t}");

    ed.cancel(false);
    assert_eq!(ed.selections()[0].head, PointUtf16::new(3, 15));
}

#[gpui::test]
fn old_stops_when_calling_set_value(cx: &mut TestAppContext) {
    let mut ed = old_setup(OLD_LINES, cx);
    ed.select(&[p(4, 2)]);
    ed.insert(OLD_TEMPLATE);
    assert!(ed.in_snippet());
    ed.set_value("goodbye");
    assert!(!ed.in_snippet());
}

#[gpui::test]
fn old_stops_when_undoing(cx: &mut TestAppContext) {
    let mut ed = old_setup(OLD_LINES, cx);
    ed.select(&[p(4, 2)]);
    ed.insert(OLD_TEMPLATE);
    assert!(ed.in_snippet());
    ed.undo();
    assert!(!ed.in_snippet());
    assert_eq!(ed.text(), OLD_LINES.join("\n"));
}

#[gpui::test]
fn old_stops_when_moving_cursor_outside(cx: &mut TestAppContext) {
    let mut ed = old_setup(OLD_LINES, cx);
    ed.select(&[p(4, 2)]);
    ed.insert(OLD_TEMPLATE);
    assert!(ed.in_snippet());
    ed.select(&[p(1, 1)]);
    assert!(!ed.in_snippet());
}

#[gpui::test]
fn old_stops_when_disconnecting_editor_model(cx: &mut TestAppContext) {
    let mut ed = old_setup(OLD_LINES, cx);
    ed.select(&[p(4, 2)]);
    ed.insert(OLD_TEMPLATE);
    assert!(ed.in_snippet());
    let other = ed.cx.new(|_| TextModel::new(""));
    ed.editor.update(ed.cx, |e, cx| e.set_model(other, cx));
    assert!(!ed.in_snippet());
}

// Skipped: `Stops when disposing editor` (there's no controller to dispose).

#[gpui::test]
fn old_final_tabstop_with_multiple_selections_1(cx: &mut TestAppContext) {
    let mut ed = old_setup(OLD_LINES, cx);
    ed.select(&[p(1, 1), p(2, 1)]);
    ed.insert("foo$0");
    ed.assert_selections(&[p(1, 4), p(2, 4)]);
}

#[gpui::test]
fn old_final_tabstop_with_multiple_selections_2(cx: &mut TestAppContext) {
    let mut ed = old_setup(OLD_LINES, cx);
    ed.select(&[p(1, 1), p(2, 1)]);
    ed.insert("foo$0bar");
    ed.assert_selections(&[p(1, 4), p(2, 4)]);
}

#[gpui::test]
fn old_final_tabstop_with_multiple_selections_3(cx: &mut TestAppContext) {
    let mut ed = old_setup(OLD_LINES, cx);
    ed.select(&[p(1, 1), p(1, 5)]);
    ed.insert("foo$0bar");
    ed.assert_selections(&[p(1, 4), p(1, 14)]);
}

#[gpui::test]
fn old_final_tabstop_with_multiple_selections_4(cx: &mut TestAppContext) {
    let mut ed = old_setup(OLD_LINES, cx);
    ed.select(&[p(1, 1), p(1, 5)]);
    ed.insert("foo\n$0\nbar");
    ed.assert_selections(&[p(2, 1), p(4, 1)]);
}

#[gpui::test]
fn old_final_tabstop_with_multiple_selections_overwrite_before(cx: &mut TestAppContext) {
    let mut ed = old_setup(OLD_LINES, cx);
    ed.select(&[p(2, 7)]);
    ed.insert_with("xo$0r", before(1));
    ed.assert_selections(&[p(2, 8)]);
}

#[gpui::test]
fn old_final_tabstop_11742_simple_1(cx: &mut TestAppContext) {
    let mut ed = old_setup(&["example example sc"], cx);
    ed.select(&[p(1, 19)]);
    ed.insert_with("{{% url_**$1** %}}", before(2));
    ed.assert_selections(&[p(1, 27)]);
    assert_eq!(ed.text(), "example example {{% url_**** %}}");
}

#[gpui::test]
fn old_final_tabstop_11742_simple_2(cx: &mut TestAppContext) {
    let mut ed = old_setup(&["af"], cx);
    ed.select(&[p(1, 3)]);
    ed.insert_with("afterEach((done) => {\n\t${1}test\n});", before(2));
    ed.assert_selections(&[p(2, 2)]);
    assert_eq!(ed.text(), "afterEach((done) => {\n\ttest\n});");
}

#[gpui::test]
fn old_final_tabstop_11742_simple_3(cx: &mut TestAppContext) {
    let mut ed = old_setup(&["af"], cx);
    ed.select(&[p(1, 3)]);
    ed.insert_with("afterEach((done) => {\n${1}\ttest\n});", before(2));
    ed.assert_selections(&[p(2, 1)]);
    assert_eq!(ed.text(), "afterEach((done) => {\n\ttest\n});");
}

#[gpui::test]
fn old_final_tabstop_11742_simple_4(cx: &mut TestAppContext) {
    let mut ed = old_setup(&["afterone"], cx);
    ed.select(&[p(1, 9)]);
    ed.insert_with("aft${1}er", before(8));
    assert_eq!(ed.text(), "after");
    ed.assert_selections(&[p(1, 4)]);
}

#[gpui::test]
fn old_final_tabstop_11742_different_indents(cx: &mut TestAppContext) {
    let mut ed = old_setup(&["af", "\taf"], cx);
    ed.select(&[p(2, 4), p(1, 3)]);
    ed.insert_with("afterEach((done) => {\n\t${0}test\n});", before(2));
    ed.assert_selections(&[p(5, 3), p(2, 2)]);
}

#[gpui::test]
fn old_final_tabstop_11890_stay_at_the_beginning(cx: &mut TestAppContext) {
    let mut ed = old_setup(&["  af"], cx);
    ed.select(&[p(1, 5)]);
    ed.insert_with("afterEach((done) => {\n${1}\ttest\n});", before(2));
    ed.assert_selections(&[p(2, 3)]);
}

#[gpui::test]
fn old_final_tabstop_no_tabstop(cx: &mut TestAppContext) {
    let mut ed = old_setup(&["af", "\taf"], cx);
    ed.select(&[p(1, 3)]);
    ed.insert_with("afterEach", before(2));
    ed.assert_selections(&[p(1, 10)]);
}

#[gpui::test]
fn old_multiple_cursor_and_overwrite_before_11060(cx: &mut TestAppContext) {
    let mut ed = old_setup(&["this._", "abc"], cx);
    // (lines, selections, template, overwrite_before, expected)
    type Case<'a> = (&'a [&'a str], &'a [Selection], &'a str, u32, &'a str);
    let cases: &[Case] = &[
        (
            &["this._", "abc"],
            &[p(1, 7), p(2, 4)],
            "_foo",
            1,
            "this._foo\nabc_foo",
        ),
        (
            &["this._", "abc"],
            &[p(1, 7), p(2, 4)],
            "XX",
            1,
            "this.XX\nabcXX",
        ),
        (
            &["this._", "abc", "def_"],
            &[p(1, 7), p(2, 4), p(3, 5)],
            "_foo",
            1,
            "this._foo\nabc_foo\ndef_foo",
        ),
        // Primary at `this._`.
        (
            &["this._", "abc", "def._"],
            &[p(1, 7), p(2, 4), p(3, 6)],
            "._foo",
            2,
            "this._foo\nabc._foo\ndef._foo",
        ),
        // Primary at `def._`.
        (
            &["this._", "abc", "def._"],
            &[p(3, 6), p(1, 7), p(2, 4)],
            "._foo",
            2,
            "this._foo\nabc._foo\ndef._foo",
        ),
        // Primary at `abc`.
        (
            &["this._", "abc", "def._"],
            &[p(2, 4), p(3, 6), p(1, 7)],
            "._foo",
            2,
            "this._._foo\na._foo\ndef._._foo",
        ),
    ];
    for (lines, selections, template, overwrite_before, expected) in cases {
        ed.set_value(&lines.join("\n"));
        ed.select(selections);
        ed.insert_with(template, before(*overwrite_before));
        assert_eq!(ed.text(), *expected, "{template:?} at {selections:?}");
    }
}

#[gpui::test]
fn old_multiple_cursor_and_overwrite_before_16277(cx: &mut TestAppContext) {
    let mut ed = old_setup(&["{foo}", "{foo && true}"], cx);
    ed.select(&[p(1, 5), p(2, 5)]);
    ed.insert_with("document", before(3));
    assert_eq!(ed.text(), "{document}\n{document && true}");
}

#[gpui::test]
fn old_insert_snippet_twice_19449(cx: &mut TestAppContext) {
    let mut ed = old_setup(&["for (var i=0; i<len; i++) {  }"], cx);
    ed.select(&[p(1, 1)]);
    ed.insert("for (var ${1:i}=0; ${1:i}<len; ${1:i}++) { $0 }");
    assert_eq!(
        ed.text(),
        "for (var i=0; i<len; i++) {  }for (var i=0; i<len; i++) {  }"
    );

    ed.cancel(false);
    ed.set_value("for (var i=0; i<len; i++) {  }");
    ed.select(&[p(1, 1)]);
    ed.insert("for (let ${1:i}=0; ${1:i}<len; ${1:i}++) { $0 }");
    assert_eq!(
        ed.text(),
        "for (let i=0; i<len; i++) {  }for (var i=0; i<len; i++) {  }"
    );
}
