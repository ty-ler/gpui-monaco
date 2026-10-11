//! Monaco `ITextModel` helpers: positions, searching with captures and
//! scopes, edits with undo edits, the undo stack, brackets and guides.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use gpui_monaco::colorize::tokenize;
use gpui_monaco::cursor::Selection;
use gpui_monaco::text::{LineEnding, PointUtf16};
use gpui_monaco::{CodeEditor, FindMatch, ModelOptions, TextModel, TextModelEvent};

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

/// Monaco `textModel.test.ts` "modifyPosition" (1-based positions there).
#[gpui::test]
fn modify_position(cx: &mut TestAppContext) {
    let model = cx.new(|_| TextModel::new("line one\nline two"));
    model.read_with(cx, |m, _| {
        let at = |row: u32, column: u32, offset: isize| m.modify_position(p(row, column), offset);
        assert_eq!(at(0, 0, 0), p(0, 0));
        assert_eq!(at(29, 0, 0), p(1, 8));
        assert_eq!(at(0, 0, 17), p(1, 8));
        assert_eq!(at(0, 0, 1), p(0, 1));
        assert_eq!(at(0, 0, 3), p(0, 3));
        assert_eq!(at(0, 1, 10), p(1, 2));
        assert_eq!(at(0, 4, 13), p(1, 8));
        assert_eq!(at(0, 1, 16), p(1, 8));
        assert_eq!(at(1, 8, -17), p(0, 0));
        assert_eq!(at(0, 1, -1), p(0, 0));
        assert_eq!(at(0, 3, -3), p(0, 0));
        assert_eq!(at(1, 2, -10), p(0, 1));
        assert_eq!(at(1, 8, -13), p(0, 4));
        assert_eq!(at(1, 8, -16), p(0, 1));
        assert_eq!(at(0, 1, 17), p(1, 8));
        assert_eq!(at(0, 1, 100), p(1, 8));
        assert_eq!(at(0, 1, -2), p(0, 0));
        assert_eq!(at(0, 1, -100), p(0, 0));
        assert_eq!(at(1, 1, -100), p(0, 0));
        assert_eq!(at(1, 8, -18), p(0, 0));
    });
}

#[gpui::test]
fn lines_lengths_and_ranges(cx: &mut TestAppContext) {
    let model = cx.new(|_| TextModel::new("a😀\r\nbc\r\n"));
    model.read_with(cx, |m, _| {
        assert_eq!(m.lines_content(), ["a😀", "bc", ""]);
        // UTF-16 units, with the CRLF line ending counted as two.
        assert_eq!(m.value_length(), 3 + 2 + 2 + 2);
        assert_eq!(m.full_model_range(), p(0, 0)..p(2, 0));
        // Offsets count the CRLF as two units.
        assert_eq!(m.modify_position(p(0, 0), 5), p(1, 0));
        assert_eq!(m.modify_position(p(1, 0), -2), p(0, 3));
    });
}

fn found(range: std::ops::Range<PointUtf16>, matches: &[&str]) -> Option<FindMatch> {
    Some(FindMatch {
        range,
        matches: Some(matches.iter().map(|m| Some(m.to_string())).collect()),
    })
}

/// Monaco `textModelSearch.test.ts`, the capturing-matches tests.
#[gpui::test]
fn find_next_and_previous_with_captures(cx: &mut TestAppContext) {
    let model = cx.new(|_| TextModel::new("one line line\ntwo line\nthree"));
    model.read_with(cx, |m, _| {
        let next = |query: &str| m.find_next_match(query, p(0, 0), true, false, None, true);
        let previous = |query: &str| m.find_previous_match(query, p(0, 0), true, false, None, true);
        assert_eq!(
            next("(l(in)e)"),
            found(p(0, 4)..p(0, 8), &["line", "line", "in"])
        );
        assert_eq!(
            next("(l(in)e)\\n"),
            found(p(0, 9)..p(1, 0), &["line\n", "line", "in"])
        );
        assert_eq!(
            previous("(l(in)e)"),
            found(p(1, 4)..p(1, 8), &["line", "line", "in"])
        );
        assert_eq!(
            previous("(l(in)e)\\n"),
            found(p(1, 4)..p(2, 0), &["line\n", "line", "in"])
        );
        // Without captures.
        assert_eq!(
            m.find_next_match("line", p(0, 5), false, false, None, false),
            Some(FindMatch {
                range: p(0, 9)..p(0, 13),
                matches: None,
            })
        );
    });
}

#[gpui::test]
fn find_matches_in_a_scope(cx: &mut TestAppContext) {
    let model = cx.new(|_| TextModel::new("foo bar foo\nfoo baz foo\nfoo"));
    model.read_with(cx, |m, _| {
        let ranges = |scope: &[std::ops::Range<PointUtf16>], limit: usize| {
            m.find_matches_in("foo", scope, false, false, None, false, limit)
                .into_iter()
                .map(|f| f.range)
                .collect::<Vec<_>>()
        };
        // Two scopes, out of order and overlapping: merged and sorted.
        assert_eq!(
            ranges(&[p(1, 0)..p(1, 5), p(0, 4)..p(1, 2), p(2, 0)..p(2, 3)], 100),
            [p(0, 8)..p(0, 11), p(1, 0)..p(1, 3), p(2, 0)..p(2, 3)]
        );
        assert_eq!(ranges(&[m.full_model_range()], 2).len(), 2);
        // Whole words and captures.
        let words = m.find_matches_in(
            "(f)oo",
            &[p(0, 0)..p(0, 11)],
            true,
            true,
            Some("`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?"),
            true,
            10,
        );
        assert_eq!(words.len(), 2);
        assert_eq!(
            words[0].matches,
            Some(vec![Some("foo".to_string()), Some("f".to_string())])
        );
    });
}

#[gpui::test]
fn apply_edits_returns_undo_edits(cx: &mut TestAppContext) {
    let model = cx.new(|_| TextModel::new("hello world\nsecond"));
    let undo = model.update(cx, |m, cx| {
        m.apply_edits(
            vec![
                (p(0, 0)..p(0, 5), "goodbye".into()),
                (p(1, 0)..p(1, 0), "a ".into()),
            ],
            cx,
        )
        .unwrap()
    });
    assert_eq!(
        model.read_with(cx, |m, _| m.text()),
        "goodbye world\na second"
    );
    assert_eq!(
        undo,
        [
            (p(0, 0)..p(0, 7), "hello".to_string()),
            (p(1, 0)..p(1, 2), String::new())
        ]
    );
    // Applying them restores the text.
    model.update(cx, |m, cx| m.apply_edits(undo, cx).unwrap());
    assert_eq!(model.read_with(cx, |m, _| m.text()), "hello world\nsecond");
}

#[gpui::test]
fn push_edit_operations_and_the_undo_stack(cx: &mut TestAppContext) {
    let model = cx.new(|_| TextModel::new(""));
    let alternative =
        |cx: &mut TestAppContext| model.read_with(cx, |m, _| m.alternative_version_id());
    let start = alternative(cx);
    // The cursor computer gets the undo edits.
    let after = model.update(cx, |m, cx| {
        m.push_edit_operations(
            Some(vec![Selection::collapsed(p(0, 0))]),
            vec![(p(0, 0)..p(0, 0), "ab".into())],
            Some(&|undo: &[(std::ops::Range<PointUtf16>, String)]| {
                Some(vec![Selection::collapsed(undo[0].0.end)])
            }),
            cx,
        )
        .unwrap()
    });
    assert_eq!(after, Some(vec![Selection::collapsed(p(0, 2))]));
    // Consecutive pushes join one undo step until push_stack_element.
    model.update(cx, |m, cx| {
        m.push_edit_operations(None, vec![(p(0, 2)..p(0, 2), "c".into())], None, cx)
            .unwrap();
        m.push_stack_element();
        m.push_edit_operations(None, vec![(p(0, 3)..p(0, 3), "d".into())], None, cx)
            .unwrap();
    });
    let typed = alternative(cx);
    model.update(cx, |m, cx| m.undo(None, cx));
    assert_eq!(model.read_with(cx, |m, _| m.text()), "abc");
    model.update(cx, |m, cx| m.undo(None, cx));
    assert_eq!(model.read_with(cx, |m, _| m.text()), "");
    // Undo returns to the alternative version the text had.
    assert_eq!(alternative(cx), start);
    model.update(cx, |m, cx| {
        m.redo(None, cx);
        m.redo(None, cx);
    });
    assert_eq!(alternative(cx), typed);
    // pop_stack_element reopens the last step.
    model.update(cx, |m, cx| {
        m.push_stack_element();
        m.edit([(4..4, "e")], cx).unwrap();
        m.pop_stack_element();
        m.edit([(5..5, "f")], cx).unwrap();
        m.undo(None, cx);
    });
    assert_eq!(model.read_with(cx, |m, _| m.text()), "abcd");
}

#[gpui::test]
fn brackets_and_guides(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
    });
    let text = "fn f() {\n    let s = \"(\";\n    if x {\n        g(1);\n    }\n}\n";
    let model = cx.new(|_| TextModel::new(text).with_language("rust"));
    // Models resolve their language when first used.
    model.update(cx, |m, cx| m.ensure_syntax(cx));
    cx.run_until_parked();
    model.read_with(cx, |m, _| {
        // The bracket in the string is ignored.
        let pairs = m.bracket_pairs_in_range(p(2, 0)..p(4, 5));
        let summary: Vec<_> = pairs
            .iter()
            .map(|pair| (pair.open.start, pair.close.start, pair.level))
            .collect();
        assert_eq!(
            summary,
            [
                (p(0, 7), p(5, 0), 0),
                (p(2, 9), p(4, 4), 1),
                (p(3, 9), p(3, 11), 2)
            ]
        );
        // Touching a bracket, then around a position.
        assert_eq!(
            m.match_bracket(p(2, 9)),
            Some((p(2, 9)..p(2, 10), p(4, 4)..p(4, 5)))
        );
        assert_eq!(m.match_bracket(p(1, 6)), None);
        assert_eq!(
            m.find_enclosing_brackets(p(3, 8)),
            Some((p(2, 9)..p(2, 10), p(4, 4)..p(4, 5)))
        );
        assert_eq!(m.lines_indent_guides(0..6), [0, 1, 1, 2, 1, 0]);
        let active = m.active_indent_guide(3, 0, 5);
        assert_eq!((active.start_row, active.end_row, active.indent), (3, 3, 2));
    });
}

fn model_with(text: &str, insert_spaces: bool, cx: &mut TestAppContext) -> Entity<TextModel> {
    cx.new(|_| {
        TextModel::new(text).with_options(ModelOptions {
            insert_spaces,
            detect_indentation: false,
            ..Default::default()
        })
    })
}

/// Monaco `textModel.test.ts` "normalizeIndentation 1".
#[gpui::test]
fn normalize_indentation_with_tabs(cx: &mut TestAppContext) {
    let model = model_with("", false, cx);
    model.read_with(cx, |m, _| {
        let n = |s: &str| m.normalize_indentation(s);
        assert_eq!(n("\t"), "\t");
        assert_eq!(n("    "), "\t");
        assert_eq!(n("   "), "   ");
        assert_eq!(n("  "), "  ");
        assert_eq!(n(" "), " ");
        assert_eq!(n(""), "");
        assert_eq!(n(" \t    "), "\t\t");
        assert_eq!(n(" \t   "), "\t   ");
        assert_eq!(n(" \t  "), "\t  ");
        assert_eq!(n(" \t "), "\t ");
        assert_eq!(n(" \t"), "\t");

        assert_eq!(n("\ta"), "\ta");
        assert_eq!(n("    a"), "\ta");
        assert_eq!(n("   a"), "   a");
        assert_eq!(n("  a"), "  a");
        assert_eq!(n(" a"), " a");
        assert_eq!(n("a"), "a");
        assert_eq!(n(" \t    a"), "\t\ta");
        assert_eq!(n(" \t   a"), "\t   a");
        assert_eq!(n(" \t  a"), "\t  a");
        assert_eq!(n(" \t a"), "\t a");
        assert_eq!(n(" \ta"), "\ta");
    });
}

/// Monaco `textModel.test.ts` "normalizeIndentation 2".
#[gpui::test]
fn normalize_indentation_with_spaces(cx: &mut TestAppContext) {
    let model = model_with("", true, cx);
    model.read_with(cx, |m, _| {
        let n = |s: &str| m.normalize_indentation(s);
        assert_eq!(n("\ta"), "    a");
        assert_eq!(n("    a"), "    a");
        assert_eq!(n("   a"), "   a");
        assert_eq!(n("  a"), "  a");
        assert_eq!(n(" a"), " a");
        assert_eq!(n("a"), "a");
        assert_eq!(n(" \t    a"), "        a");
        assert_eq!(n(" \t   a"), "       a");
        assert_eq!(n(" \t  a"), "      a");
        assert_eq!(n(" \t a"), "     a");
        assert_eq!(n(" \ta"), "    a");
    });
}

/// Monaco `textModel.test.ts` "getLineFirstNonWhitespaceColumn" and
/// "getLineLastNonWhitespaceColumn" (1-based, `0` for none, there).
#[gpui::test]
fn first_and_last_non_whitespace_columns(cx: &mut TestAppContext) {
    let lines = [
        "asd", " asd", "\tasd", "  asd", "\t\tasd", " ", "  ", "\t", "\t\t", "  \tasd", "", "",
    ];
    let model = model_with(&lines.join("\n"), true, cx);
    model.read_with(cx, |m, _| {
        let columns: Vec<_> = (0..12)
            .map(|row| m.line_first_non_whitespace_column(row))
            .collect();
        let none = None;
        assert_eq!(
            columns,
            [
                Some(0),
                Some(1),
                Some(1),
                Some(2),
                Some(2),
                none,
                none,
                none,
                none,
                Some(3),
                none,
                none
            ]
        );
    });
    let lines = [
        "asd", "asd ", "asd\t", "asd  ", "asd\t\t", " ", "  ", "\t", "\t\t", "asd  \t", "", "",
    ];
    let model = model_with(&lines.join("\n"), true, cx);
    model.read_with(cx, |m, _| {
        let columns: Vec<_> = (0..12)
            .map(|row| m.line_last_non_whitespace_column(row))
            .collect();
        let (three, none) = (Some(3), None);
        assert_eq!(
            columns,
            [
                three, three, three, three, three, none, none, none, none, three, none, none
            ]
        );
    });
}

/// Monaco `getLineIndentColumn` and `computeIndentLevel`.
#[gpui::test]
fn indent_columns_and_levels(cx: &mut TestAppContext) {
    let model = model_with("  \tx\n\t  y\n   \nz", true, cx);
    model.read_with(cx, |m, _| {
        // Each space or tab is one column.
        assert_eq!(m.line_indent_column(0), 3);
        assert_eq!(m.line_indent_column(1), 3);
        assert_eq!(m.line_indent_column(3), 0);
        // Tabs go to the next tab stop (4); whitespace-only lines have none.
        assert_eq!(m.line_indent_level(0), Some(4));
        assert_eq!(m.line_indent_level(1), Some(6));
        assert_eq!(m.line_indent_level(2), None);
        assert_eq!(m.line_indent_level(3), Some(0));
    });
}

/// Monaco `pushEOL`: the line ending change is an undo step, and undo and
/// redo report it.
#[gpui::test]
fn push_line_ending_is_undoable(cx: &mut TestAppContext) {
    let model = cx.new(|_| TextModel::new("Hello\nworld"));
    let changes = Rc::new(Cell::new(0));
    let seen = changes.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&model, move |_, event: &TextModelEvent, _| {
            if matches!(event, TextModelEvent::LineEndingChanged) {
                seen.set(seen.get() + 1);
            }
        })
    });
    let version = model.read_with(cx, |m, _| m.version());
    model.update(cx, |m, cx| m.push_line_ending(LineEnding::Crlf, cx));
    assert_eq!(model.read_with(cx, |m, _| m.text()), "Hello\r\nworld");
    // The same line ending again does nothing.
    model.update(cx, |m, cx| m.push_line_ending(LineEnding::Crlf, cx));
    assert_eq!(changes.get(), 1);
    assert_eq!(model.read_with(cx, |m, _| m.version()), version);
    assert!(model.read_with(cx, |m, _| m.can_undo()));
    model.update(cx, |m, cx| m.undo(None, cx));
    assert_eq!(model.read_with(cx, |m, _| m.text()), "Hello\nworld");
    assert_eq!(changes.get(), 2);
    model.update(cx, |m, cx| m.redo(None, cx));
    assert_eq!(model.read_with(cx, |m, _| m.text()), "Hello\r\nworld");
    assert_eq!(changes.get(), 3);
    // `set_line_ending` (Monaco `setEOL`) isn't an undo step.
    model.update(cx, |m, cx| {
        m.set_line_ending(LineEnding::Lf, cx);
        m.undo(None, cx);
    });
    assert_eq!(model.read_with(cx, |m, _| m.text()), "Hello\nworld");
}

/// Monaco `tokenization.getLineTokens`: a line's tokens, with injected
/// languages.
#[gpui::test]
fn line_tokens(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry()));
    let text = "fn main() {\n\tlet x = 1;\n}";
    let model = cx.new(|_| TextModel::new(text).with_language("rust"));
    model.update(cx, |m, cx| m.ensure_syntax(cx));
    cx.run_until_parked();
    let tokens = model.read_with(cx, |m, cx| m.line_tokens(1, cx)).unwrap();
    // The same tokens as `tokenize`.
    let registry = gpui_monaco_languages::registry();
    assert_eq!(tokens, tokenize(text, "rust", &registry)[1]);
    assert_eq!(tokens[1].offset, 1);
    assert_eq!(tokens[1].token_type.as_ref(), "keyword.rust");
    assert!(model.read_with(cx, |m, cx| m.line_tokens(3, cx)).is_none());
    // An edit not parsed yet is tokenized right away.
    model.update(cx, |m, cx| m.edit([(0..0, "pub ")], cx).unwrap());
    let tokens = model.read_with(cx, |m, cx| m.line_tokens(0, cx)).unwrap();
    assert_eq!(tokens[0].token_type.as_ref(), "keyword.rust");
    assert!(
        tokens
            .iter()
            .any(|t| t.offset == 4 && t.token_type.as_ref() == "keyword.rust")
    );

    // A Rust code fence in Markdown has Rust tokens.
    let markdown = "# Title\n```rust\nfn f() {}\n```\n";
    let model = cx.new(|_| TextModel::new(markdown).with_language("markdown"));
    model.update(cx, |m, cx| m.ensure_syntax(cx));
    cx.run_until_parked();
    let tokens = model.read_with(cx, |m, cx| m.line_tokens(2, cx)).unwrap();
    assert_eq!(tokens[0].token_type.as_ref(), "keyword.rust");
    assert_eq!(tokens[0].language.as_ref(), "rust");
    // A model without a grammar has one token per line.
    let plain = cx.new(|_| TextModel::new("plain text"));
    let tokens = plain.read_with(cx, |m, cx| m.line_tokens(0, cx)).unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!((tokens[0].offset, tokens[0].token_type.as_ref()), (0, ""));
}

/// Monaco `isAttachedToEditor` and `onDidChangeAttached`: editors attach
/// their model, and detach it when they show another or go away.
#[gpui::test]
fn attached_editors(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let model = cx.new(|_| TextModel::new("text"));
    let other = cx.new(|_| TextModel::new("other"));
    let changes = Rc::new(Cell::new(0));
    let seen = changes.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&model, move |_, event: &TextModelEvent, _| {
            if matches!(event, TextModelEvent::AttachedChanged) {
                seen.set(seen.get() + 1);
            }
        })
    });
    assert!(!model.read_with(cx, |m, _| m.is_attached_to_editor()));
    let (first, cx) = cx.add_window_view(|window, cx| CodeEditor::new(model.clone(), window, cx));
    let attached = |cx: &mut VisualTestContext| {
        model.read_with(cx, |m, _| {
            (m.is_attached_to_editor(), m.attached_editor_count())
        })
    };
    assert_eq!(attached(cx), (true, 1));
    assert_eq!(changes.get(), 1);
    // A second editor: no change event.
    let second = cx.update(|window, cx| cx.new(|cx| CodeEditor::new(model.clone(), window, cx)));
    assert_eq!(attached(cx), (true, 2));
    assert_eq!(changes.get(), 1);
    second.update(cx, |e, cx| e.set_model(other.clone(), cx));
    assert_eq!(attached(cx), (true, 1));
    assert!(other.read_with(cx, |m, _| m.is_attached_to_editor()));
    // The last editor showing it leaves.
    first.update(cx, |e, cx| e.set_model(other.clone(), cx));
    assert_eq!(attached(cx), (false, 0));
    assert_eq!(changes.get(), 2);
    // Dropping an editor detaches its model.
    assert_eq!(other.read_with(cx, |m, _| m.attached_editor_count()), 2);
    drop(second);
    cx.run_until_parked();
    assert_eq!(other.read_with(cx, |m, _| m.attached_editor_count()), 1);
}

/// Monaco `cursor.test.ts` "issue #23539: Setting model EOL isn't
/// undoable" and "can undo typing and EOL change in one undo stop".
#[gpui::test]
fn undo_line_ending_with_typing(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let model = cx.new(|_| TextModel::new("Hello\nworld"));
    let (editor, cx) = cx.add_window_view(|window, cx| CodeEditor::new(model.clone(), window, cx));
    model.update(cx, |m, cx| {
        m.set_line_ending(LineEnding::Lf, cx);
        m.push_line_ending(LineEnding::Crlf, cx);
    });
    assert_eq!(model.read_with(cx, |m, _| m.text()), "Hello\r\nworld");
    editor.update(cx, |e, cx| e.undo(cx));
    assert_eq!(model.read_with(cx, |m, _| m.text()), "Hello\nworld");

    let model = cx.update(|_, cx| cx.new(|_| TextModel::new("A  line\nAnother line")));
    editor.update(cx, |e, cx| {
        e.set_model(model.clone(), cx);
        e.set_selections(&[Selection::collapsed(p(0, 2))], cx);
        e.type_text("first", cx);
    });
    assert_eq!(
        model.read_with(cx, |m, _| m.text()),
        "A first line\nAnother line"
    );
    model.update(cx, |m, cx| m.push_line_ending(LineEnding::Crlf, cx));
    assert_eq!(
        model.read_with(cx, |m, _| m.text()),
        "A first line\r\nAnother line"
    );
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()),
        [Selection::collapsed(p(0, 7))]
    );
    editor.update(cx, |e, cx| e.undo(cx));
    assert_eq!(
        model.read_with(cx, |m, _| m.text()),
        "A  line\nAnother line"
    );
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()),
        [Selection::collapsed(p(0, 2))]
    );
}
