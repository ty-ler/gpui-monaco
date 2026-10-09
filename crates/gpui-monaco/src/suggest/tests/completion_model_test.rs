//! Port of Monaco's
//! `src/vs/editor/contrib/suggest/test/browser/completionModel.test.ts`
//! (MIT), suite `CompletionModel`.
//!
//! Monaco items carry their own 1-based position; every test puts the items
//! and the model at the same position, so here they all sit at
//! [`SESSION`] and only `overwriteBefore` (the range length before the
//! cursor) matters.
//!
//! Adapted:
//! - `filtering - cached` checks results, not object identity.
//! - `Fuzzy matching of snippets stopped working with inline snippet
//!   suggestions #49895` checks only the item count (incomplete providers
//!   aren't tracked by the model).
//! - `keep snippet sorting with prefix: *` compare the numeric score where
//!   Monaco compares score arrays.
//! - `Completion item sorting broken when using label details #153026`
//!   checks the provider-order comparator through the model's order for an
//!   empty word (labels with details aren't modelled).
//!
//! Skipped (by reason):
//! - `complete/incomplete`: incomplete providers are tracked by the suggest
//!   session, not the model.

use text::PointUtf16;

use crate::providers::{CompletionItem, CompletionItemKind};
use crate::suggest::completion_model::{
    CompletionModel, CompletionModelOptions, LineContext, SnippetSortOrder,
};

const SESSION: PointUtf16 = PointUtf16 { row: 0, column: 20 };

/// Monaco `createSuggestItem`.
fn item(label: &str, overwrite_before: u32) -> CompletionItem {
    item_with(
        label,
        overwrite_before,
        CompletionItemKind::Property,
        None,
        None,
    )
}

fn item_with(
    label: &str,
    overwrite_before: u32,
    kind: CompletionItemKind,
    sort_text: Option<&str>,
    filter_text: Option<&str>,
) -> CompletionItem {
    let start = PointUtf16::new(SESSION.row, SESSION.column - overwrite_before);
    CompletionItem {
        sort_text: sort_text.map(|s| s.to_string().into()),
        filter_text: filter_text.map(|s| s.to_string().into()),
        range: Some(start..SESSION),
        insert_text: Some(label.to_string().into()),
        ..CompletionItem::new(label.to_string(), kind)
    }
}

fn ctx(leading: &str, delta: usize) -> LineContext {
    LineContext {
        leading_line_content: leading.to_string(),
        character_count_delta: delta,
    }
}

fn model(items: Vec<CompletionItem>, line_context: LineContext) -> CompletionModel {
    CompletionModel::new(items, SESSION, SESSION.column, line_context)
}

fn model_with_snippets(
    items: Vec<CompletionItem>,
    line_context: LineContext,
    snippet_sort_order: SnippetSortOrder,
) -> CompletionModel {
    CompletionModel::new_with_options(
        items,
        SESSION,
        SESSION.column,
        line_context,
        CompletionModelOptions {
            snippet_sort_order,
            ..CompletionModelOptions::default()
        },
    )
}

fn labels(model: &CompletionModel) -> Vec<String> {
    model
        .filtered()
        .iter()
        .map(|s| model.items()[s.index].label.to_string())
        .collect()
}

fn setup() -> CompletionModel {
    model(
        vec![item("foo", 3), item("Foo", 3), item("foo", 2)],
        ctx("foo", 0),
    )
}

#[test]
fn filtering_cached() {
    let mut model = setup();
    let before = labels(&model);
    // `foo` with overwriteBefore 2 scores `oo` against `foo`: a weak first
    // match, filtered out.
    assert_eq!(before, ["foo", "Foo"]);

    // still the same context
    model.set_line_context(ctx("foo", 0));
    assert_eq!(labels(&model), before);

    // different context, refilter
    model.set_line_context(ctx("foo1", 1));
    assert_ne!(labels(&model), before);
    assert!(model.filtered().is_empty());
}

#[test]
fn fuzzy_matching_of_snippets_with_inline_snippet_suggestions_49895() {
    let model = model(
        vec![
            item("foobar1", 1),
            item("foobar2", 1),
            item("foobar3", 1),
            item("foobar4", 1),
            item("foobar5", 1),
            item("foofoo1", 1),
        ],
        ctx("f", 0),
    );
    assert_eq!(model.filtered().len(), 6);
}

#[test]
fn proper_current_word_when_length_0_16380() {
    let model = model(
        vec![
            item("    </div", 4),
            item("a", 0),
            item("p", 0),
            item("    </tag", 4),
            item("    XYZ", 4),
        ],
        ctx("   <", 0),
    );
    assert_eq!(labels(&model), ["    </div", "    </tag", "a", "p"]);
}

#[test]
fn keep_snippet_sorting_with_prefix_top_25495() {
    let model = model_with_snippets(
        vec![
            item_with("Snippet1", 1, CompletionItemKind::Snippet, None, None),
            item_with("tnippet2", 1, CompletionItemKind::Snippet, None, None),
            item_with("semver", 1, CompletionItemKind::Property, None, None),
        ],
        ctx("s", 0),
        SnippetSortOrder::Top,
    );
    assert_eq!(labels(&model), ["Snippet1", "semver"]);
    let [a, b] = model.filtered() else { panic!() };
    assert!(a.score.score < b.score.score); // snippet really promoted
}

#[test]
fn keep_snippet_sorting_with_prefix_bottom_25495() {
    let model = model_with_snippets(
        vec![
            item_with("snippet1", 1, CompletionItemKind::Snippet, None, None),
            item_with("tnippet2", 1, CompletionItemKind::Snippet, None, None),
            item_with("Semver", 1, CompletionItemKind::Property, None, None),
        ],
        ctx("s", 0),
        SnippetSortOrder::Bottom,
    );
    assert_eq!(labels(&model), ["Semver", "snippet1"]);
    let [a, b] = model.filtered() else { panic!() };
    assert!(a.score.score < b.score.score); // snippet really demoted
}

#[test]
fn keep_snippet_sorting_with_prefix_inline_25495() {
    let model = model_with_snippets(
        vec![
            item_with("snippet1", 1, CompletionItemKind::Snippet, None, None),
            item_with("tnippet2", 1, CompletionItemKind::Snippet, None, None),
            item("Semver", 1),
        ],
        ctx("s", 0),
        SnippetSortOrder::Inline,
    );
    assert_eq!(labels(&model), ["snippet1", "Semver"]);
    let [a, b] = model.filtered() else { panic!() };
    assert!(a.score.score > b.score.score);
}

#[test]
fn filter_text_seems_ignored_26874() {
    let item1 = item_with(
        "Map - java.util",
        1,
        CompletionItemKind::Property,
        None,
        Some("Map"),
    );
    let item2 = item("Map - java.util", 1);
    let mut model = model(vec![item1, item2], ctx("M", 0));
    assert_eq!(model.filtered().len(), 2);

    model.set_line_context(ctx("Map ", 3));
    assert_eq!(model.filtered().len(), 1);
}

#[test]
fn sort_text_from_language_server_26096() {
    let item1 = item_with(
        "<- groups",
        2,
        CompletionItemKind::Property,
        Some("00002"),
        Some("  groups"),
    );
    let item2 = item_with(
        "source",
        0,
        CompletionItemKind::Property,
        Some("00001"),
        Some("source"),
    );
    let model = model(vec![item1, item2], ctx("  ", 0));
    assert_eq!(labels(&model), ["source", "<- groups"]);
}

#[test]
fn completion_item_sorting_with_label_details_153026() {
    let model = model(
        vec![
            item_with("ZZZ", 0, CompletionItemKind::Operator, None, None),
            item_with("AAA", 0, CompletionItemKind::Operator, None, None),
            item_with("III", 0, CompletionItemKind::Operator, None, None),
        ],
        ctx("", 0),
    );
    assert_eq!(labels(&model), ["AAA", "III", "ZZZ"]);
}

#[test]
fn score_only_filtered_items_when_typing_more() {
    let mut model = model(
        vec![
            item("console", 0),
            item("co_new", 0),
            item("bar", 0),
            item("car", 0),
            item("foo", 0),
        ],
        ctx("", 0),
    );
    assert_eq!(model.filtered().len(), 5);

    // narrow down once
    model.set_line_context(ctx("c", 1));
    assert_eq!(model.filtered().len(), 3);

    // query gets longer, narrow down the narrowed-down set from before
    model.set_line_context(ctx("cn", 2));
    assert_eq!(model.filtered().len(), 2);

    // query gets shorter, refilter everything
    model.set_line_context(ctx("", 0));
    assert_eq!(model.filtered().len(), 5);
}

#[test]
fn more_relaxed_suggest_matching_15419() {
    let mut model = model(
        vec![
            item("result", 0),
            item("replyToUser", 0),
            item("randomLolut", 0),
            item("car", 0),
            item("foo", 0),
        ],
        ctx("", 0),
    );
    model.set_line_context(ctx("rlut", 4));
    // Monaco expects `result, replyToUser, randomLolut`: the last two tie
    // (-5), and ties keep the provider order, which Monaco's test passes
    // unsorted. Monaco sorts provider results by label before building the
    // model (`provideSuggestionItems`), and so does this port.
    assert_eq!(labels(&model), ["result", "randomLolut", "replyToUser"]);
    let scores: Vec<i32> = model.filtered().iter().map(|s| s.score.score).collect();
    assert_eq!(scores[1], scores[2]);
}

#[test]
fn emmet_suggestion_at_top_in_jsx_39518() {
    let mut model = model(
        vec![
            item("from", 0),
            item("form", 0),
            item("form:get", 0),
            item("testForeignMeasure", 0),
            item("fooRoom", 0),
        ],
        ctx("", 0),
    );
    model.set_line_context(ctx("form", 4));
    let labels = labels(&model);
    assert_eq!(labels.len(), 5);
    assert_eq!(labels[..3], ["form", "form:get", "from"]);
}

// ----- not from Monaco ------------------------------------------------------------

#[test]
fn default_range_uses_word_start_column() {
    let items = vec![
        CompletionItem::new("print", CompletionItemKind::Function),
        CompletionItem::new("parse", CompletionItemKind::Function),
    ];
    // `pri|` typed, word starts 3 columns before the cursor.
    let model = CompletionModel::new(items, PointUtf16::new(0, 7), 4, ctx("    pri", 0));
    assert_eq!(labels(&model), ["print"]);
    assert_eq!(model.filtered()[0].highlights, vec![0..3]);
}

#[test]
fn invalid_ranges_are_skipped() {
    let mut multi_line = item("multi", 1);
    multi_line.range = Some(PointUtf16::new(0, 19)..PointUtf16::new(1, 0));
    let mut other_line = item("other", 1);
    other_line.range = Some(PointUtf16::new(3, 19)..PointUtf16::new(3, 20));
    let model = model(
        vec![multi_line, other_line, item("ok", 1), item("", 0)],
        ctx("o", 0),
    );
    assert_eq!(labels(&model), ["ok"]);
}

#[test]
fn filter_text_ranks_label_highlights() {
    let model = model(
        vec![item_with(
            "Map - java.util",
            1,
            CompletionItemKind::Class,
            None,
            Some("jMap"),
        )],
        ctx("M", 0),
    );
    let [scored] = model.filtered() else { panic!() };
    // Scored against the filterText, highlighted in the label.
    assert_eq!(scored.score.matches, vec![1..2]);
    assert_eq!(scored.highlights, vec![0..1]);
}

#[test]
fn preselect_picks_first_preselected_top_item() {
    let mut a = item("fooA", 1);
    a.preselect = false;
    let mut b = item("fooB", 1);
    b.preselect = true;
    let top = model(vec![a, b], ctx("f", 0));
    assert_eq!(labels(&top), ["fooA", "fooB"]);
    assert_eq!(top.preselected_index(), 1);

    // A preselected item outside the top score group isn't picked.
    let mut low = item("x_foo", 1);
    low.preselect = true;
    let lower = model(vec![item("foo", 1), low], ctx("f", 0));
    assert_eq!(labels(&lower), ["foo", "x_foo"]);
    assert_eq!(lower.preselected_index(), 0);
}
