//! Monaco `ITextModel` details (#353): `validateRange` around surrogate
//! pairs, `isValidRange` and the line column helpers, edit order and
//! `forceMoveMarkers` in `pushEditOperations`, words and language ids in
//! embedded languages, and uri normalization for models and markers.

use std::sync::Arc;

use gpui::{AppContext as _, Entity, TestAppContext};
use gpui_monaco::syntax::{Language, LanguageConfiguration, LanguageRegistry, Pattern};
use gpui_monaco::text::{PointUtf16, Stickiness};
use gpui_monaco::{
    DecorationOptions, Marker, MarkerFilter, MarkerSeverity, SingleEditOperation, TextModel,
};
use serde_json::json;

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

/// Monaco `textModel.test.ts` "validateRange around high-low surrogate
/// pairs 1" and "2" (1-based ranges there): an end inside a pair moves
/// after it, a start before it, and an empty range inside one moves left.
#[gpui::test]
fn validate_range_around_surrogate_pairs(cx: &mut TestAppContext) {
    let r = |sc: u32, ec: u32| p(0, sc - 1)..p(0, ec - 1);
    let model = cx.new(|_| TextModel::new("a📚b"));
    model.read_with(cx, |m, _| {
        for (range, expected) in [
            (r(1, 1), r(1, 1)),
            (r(1, 2), r(1, 2)),
            (r(1, 3), r(1, 4)),
            (r(1, 4), r(1, 4)),
            (r(1, 5), r(1, 5)),
            (r(2, 2), r(2, 2)),
            (r(2, 3), r(2, 4)),
            (r(2, 4), r(2, 4)),
            (r(2, 5), r(2, 5)),
            (r(3, 3), r(2, 2)),
            (r(3, 4), r(2, 4)),
            (r(3, 5), r(2, 5)),
            (r(4, 4), r(4, 4)),
            (r(4, 5), r(4, 5)),
            (r(5, 5), r(5, 5)),
        ] {
            assert_eq!(m.validate_range(range.clone()), expected, "{range:?}");
        }
        // The audit's repro: a range ending inside the emoji takes all of it.
        assert_eq!(
            m.value_in_range(p(0, 0)..p(0, 2), Default::default()),
            "a📚"
        );
    });
    let model = cx.new(|_| TextModel::new("a📚📚b"));
    model.read_with(cx, |m, _| {
        for (range, expected) in [
            (r(1, 1), r(1, 1)),
            (r(1, 2), r(1, 2)),
            (r(1, 3), r(1, 4)),
            (r(1, 4), r(1, 4)),
            (r(1, 5), r(1, 6)),
            (r(1, 6), r(1, 6)),
            (r(1, 7), r(1, 7)),
            (r(2, 2), r(2, 2)),
            (r(2, 3), r(2, 4)),
            (r(2, 4), r(2, 4)),
            (r(2, 5), r(2, 6)),
            (r(2, 6), r(2, 6)),
            (r(2, 7), r(2, 7)),
            (r(3, 3), r(2, 2)),
            (r(3, 4), r(2, 4)),
            (r(3, 5), r(2, 6)),
            (r(3, 6), r(2, 6)),
            (r(3, 7), r(2, 7)),
            (r(4, 4), r(4, 4)),
            (r(4, 5), r(4, 6)),
            (r(4, 6), r(4, 6)),
            (r(4, 7), r(4, 7)),
            (r(5, 5), r(4, 4)),
            (r(5, 6), r(4, 6)),
            (r(5, 7), r(4, 7)),
            (r(6, 6), r(6, 6)),
            (r(6, 7), r(6, 7)),
            (r(7, 7), r(7, 7)),
        ] {
            assert_eq!(m.validate_range(range.clone()), expected, "{range:?}");
        }
        // Reversed and out-of-range ends are ordered and clamped first.
        assert_eq!(m.validate_range(p(0, 9)..p(0, 2)), p(0, 1)..p(0, 6));
        assert_eq!(m.validate_range(p(5, 0)..p(5, 3)), p(0, 6)..p(0, 6));
    });
}

/// Monaco `isValidRange`, `getLineMinColumn` and `getLineMaxColumn`.
#[gpui::test]
fn valid_ranges_and_line_columns(cx: &mut TestAppContext) {
    let model = cx.new(|_| TextModel::new("a📚b\nxy"));
    model.read_with(cx, |m, _| {
        assert!(m.is_valid_range(&(p(0, 0)..p(0, 4))));
        assert!(m.is_valid_range(&(p(0, 1)..p(1, 2))));
        assert!(m.is_valid_range(&(p(0, 3)..p(0, 3))));
        // Inside the surrogate pair.
        assert!(!m.is_valid_range(&(p(0, 0)..p(0, 2))));
        assert!(!m.is_valid_range(&(p(0, 2)..p(0, 3))));
        // Past a line's end or the last line, or reversed.
        assert!(!m.is_valid_range(&(p(0, 0)..p(0, 5))));
        assert!(!m.is_valid_range(&(p(0, 0)..p(2, 0))));
        assert!(!m.is_valid_range(&(p(1, 1)..p(0, 1))));

        assert_eq!(m.line_min_column(0), 0);
        assert_eq!(m.line_max_column(0), 4);
        assert_eq!(m.line_min_column(1), 0);
        assert_eq!(m.line_max_column(1), 2);
    });
}

/// Monaco `model.test.ts` "issue #46342: Maintain edit operation order in
/// applyEdits": the undo edits come in the order of the edits.
#[gpui::test]
fn apply_edits_keeps_the_edit_order(cx: &mut TestAppContext) {
    let model = cx.new(|_| TextModel::new("first\nsecond"));
    let undo = model.update(cx, |m, cx| {
        m.apply_edits(
            vec![
                (p(1, 0)..p(1, 0), "a".into()),
                (p(0, 0)..p(0, 0), "b".into()),
            ],
            cx,
        )
        .unwrap()
    });
    assert_eq!(
        undo.iter()
            .map(|(range, _)| range.clone())
            .collect::<Vec<_>>(),
        [p(1, 0)..p(1, 1), p(0, 0)..p(0, 1)]
    );
}

/// Monaco `pushEditOperations` with `forceMoveMarkers` (§G.19).
#[gpui::test]
fn push_edit_operations_force_move_markers(cx: &mut TestAppContext) {
    let model = cx.new(|_| TextModel::new("hello world"));
    let never_grows = DecorationOptions {
        stickiness: Stickiness::NeverGrowsWhenTypingAtEdges,
        ..Default::default()
    };
    let ids = model.update(cx, |m, cx| {
        m.delta_decorations_at(
            None,
            &[],
            vec![
                (p(0, 0)..p(0, 5), never_grows.clone()),
                (p(0, 6)..p(0, 11), never_grows),
            ],
            cx,
        )
    });
    model.update(cx, |m, cx| {
        m.push_edit_operations(
            None,
            vec![
                SingleEditOperation::new(p(0, 5)..p(0, 5), "!").force_move_markers(),
                (p(0, 11)..p(0, 11), "?").into(),
            ],
            None,
            cx,
        )
        .unwrap()
    });
    model.read_with(cx, |m, _| {
        assert_eq!(m.text(), "hello! world?");
        // Forced, the decoration ending at the edit grows past its text.
        assert_eq!(m.decoration_range(ids[0]), Some(p(0, 0)..p(0, 6)));
        // Not forced, it doesn't.
        assert_eq!(m.decoration_range(ids[1]), Some(p(0, 7)..p(0, 12)));
    });
    // Both joined one undo step.
    model.update(cx, |m, cx| m.undo(None, cx));
    assert_eq!(model.read_with(cx, |m, _| m.text()), "hello world");
}

/// Monaco `model.test.ts` "Get word at position" (1-based columns there).
#[gpui::test]
fn word_at_position(cx: &mut TestAppContext) {
    let model = cx.new(|_| TextModel::new("This text has some  words. "));
    model.read_with(cx, |m, _| {
        let word = |column: u32| {
            m.word_at_position(p(0, column - 1))
                .map(|w| (w.word, w.range.start.column + 1, w.range.end.column + 1))
        };
        let some = |word: &str, start, end| Some((word.to_string(), start, end));
        assert_eq!(word(1), some("This", 1, 5));
        assert_eq!(word(2), some("This", 1, 5));
        assert_eq!(word(4), some("This", 1, 5));
        assert_eq!(word(5), some("This", 1, 5));
        assert_eq!(word(6), some("text", 6, 10));
        assert_eq!(word(19), some("some", 15, 19));
        assert_eq!(word(20), None);
        assert_eq!(word(21), some("words", 21, 26));
        assert_eq!(word(26), some("words", 21, 26));
        assert_eq!(word(27), None);
        assert_eq!(word(28), None);
    });
}

/// `outer` embeds `inner` between `<` and `>` (a Monarch `nextEmbedded`);
/// `inner`'s words may contain `-`.
fn embedding(text: &str, cx: &mut TestAppContext) -> Entity<TextModel> {
    let registry = Arc::new(LanguageRegistry::new());
    registry.register(Language::new("outer", "Outer"));
    registry.register(
        Language::new("inner", "Inner")
            .with_config(LanguageConfiguration {
                word_pattern: Some(Pattern::new(r"[a-z]+(-[a-z]+)*", "")),
                ..Default::default()
            })
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
    let model = cx.new(|_| TextModel::new(text).with_language("outer"));
    model.update(cx, |model, cx| model.ensure_syntax(cx));
    cx.run_until_parked();
    model
}

/// Monaco `model.test.ts` "getWordAtPosition at embedded language
/// boundaries" (1-based columns there; `x` is the inner language), and the
/// embedded language's word pattern (§G.21).
#[gpui::test]
fn word_at_position_in_embedded_languages(cx: &mut TestAppContext) {
    let model = embedding("ab<xx>ab<x>\na-b <a-b> a-b", cx);
    model.read_with(cx, |m, _| {
        let word = |row: u32, column: u32| {
            m.word_at_position(p(row, column))
                .map(|w| (w.word, w.range.start.column, w.range.end.column))
        };
        let some = |word: &str, start, end| Some((word.to_string(), start, end));
        assert_eq!(word(0, 0), some("ab", 0, 2));
        assert_eq!(word(0, 1), some("ab", 0, 2));
        assert_eq!(word(0, 2), some("ab", 0, 2));
        assert_eq!(word(0, 3), some("xx", 3, 5));
        assert_eq!(word(0, 4), some("xx", 3, 5));
        // At the boundary, the word before it in the embedded language.
        assert_eq!(word(0, 5), some("xx", 3, 5));
        assert_eq!(word(0, 6), some("ab", 6, 8));

        // `inner`'s word pattern applies only inside it.
        assert_eq!(word(1, 0), some("a", 0, 1));
        assert_eq!(word(1, 6), some("a-b", 5, 8));
        assert_eq!(word(1, 8), some("a-b", 5, 8));
        assert_eq!(word(1, 12), some("b", 12, 13));
        let until = m.word_until_position(p(1, 7));
        assert_eq!((until.word.as_str(), until.range), ("a-", p(1, 5)..p(1, 7)));
    });
}

/// Monaco `getLanguageIdAtPosition`: the language of the character after
/// the position, the last one at a line's end (§G.22).
#[gpui::test]
fn language_id_at_position(cx: &mut TestAppContext) {
    let model = embedding("a <12> b\n<3", cx);
    model.read_with(cx, |m, _| {
        let id = |row, column| m.language_id_at_position(p(row, column)).to_string();
        assert_eq!(id(0, 0), "outer");
        assert_eq!(id(0, 2), "outer");
        assert_eq!(id(0, 3), "inner");
        assert_eq!(id(0, 4), "inner");
        assert_eq!(id(0, 5), "outer");
        assert_eq!(id(0, 8), "outer");
        assert_eq!(id(1, 1), "inner");
        // Clamped to the text: the last character's.
        assert_eq!(id(9, 9), "inner");
    });
    let plain = cx.new(|_| TextModel::new("text"));
    plain.read_with(cx, |m, _| {
        assert_eq!(m.language_id_at_position(p(0, 1)).as_ref(), "plaintext");
    });
}

/// Monaco `getModel(uri)` and the marker service compare normalized uris
/// (`URI.toString()`), so spellings of one uri find the same model and
/// markers (§G.14).
#[gpui::test]
fn uris_are_normalized(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let model = cx
        .update(|cx| gpui_monaco::create_model(cx, "x", None, Some("file:///C:/a b.txt")))
        .unwrap();
    model.read_with(cx, |m, _| {
        // The uri stays as given; the resource is Monaco's form.
        assert_eq!(m.uri().map(|u| u.as_ref()), Some("file:///C:/a b.txt"));
        assert_eq!(m.resource().as_ref(), "file:///c%3A/a%20b.txt");
    });
    for spelling in [
        "file:///C:/a b.txt",
        "file:///c:/a%20b.txt",
        "file:///c%3A/a%20b.txt",
    ] {
        let found = cx.update(|cx| gpui_monaco::model_for_uri(cx, spelling));
        assert_eq!(found.as_ref(), Some(&model), "{spelling}");
    }
    // Another spelling is the same model to `create_model`.
    assert!(
        cx.update(|cx| gpui_monaco::create_model(cx, "y", None, Some("file:///c:/a%20b.txt")))
            .is_err()
    );

    model.update(cx, |m, cx| {
        m.set_markers(
            "owner",
            vec![Marker {
                severity: MarkerSeverity::Error,
                message: "boom".into(),
                range: p(0, 0)..p(0, 1),
                ..Default::default()
            }],
            cx,
        )
    });
    let markers = cx.update(|cx| {
        gpui_monaco::model_markers(
            cx,
            &MarkerFilter {
                resource: Some("file:///C:/a%20b.txt".into()),
                ..Default::default()
            },
        )
    });
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0].resource.as_ref(), "file:///c%3A/a%20b.txt");
}
