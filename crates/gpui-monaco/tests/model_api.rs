//! Monaco `ITextModel` helpers: positions, searching with captures and
//! scopes, edits with undo edits, the undo stack, brackets and guides.

use gpui::{AppContext as _, TestAppContext};
use gpui_monaco::cursor::Selection;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{FindMatch, TextModel};

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
