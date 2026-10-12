//! Port of Monaco's
//! `src/vs/editor/test/browser/controller/columnSelectionPaste.test.ts`
//! (MIT), suite `Column selection paste`.
//!
//! Skipped (by reason), each covered by a GPUI test in
//! `tests/editor.rs` where it applies:
//! - Editor and view: `uses successive model lines when word wrap is
//!   enabled`, `does not create cursors or truncate at the multi-cursor
//!   limit`, `does not edit a read-only document`, `reports pasted ranges
//!   for cursors and multiline destination selections`, `changes only the
//!   row ranges in one content change` and `responds to spread, full, and
//!   spread configuration transitions` (editor options and events).
//! - `preserves CRLF line endings`: the cursor core works on `\n`; the
//!   model joins lines with its line ending.
//! - The `clipboard metadata` suite: the editor's clipboard, tested in
//!   `tests/editor.rs`.

use super::harness::{TestEditor, sel};
use crate::cursor::{CursorConfig, MultiCursorPaste, Selection};

const BLOCK: &str = "A\nBC\nD";

fn split_lines(text: &str) -> Vec<String> {
    text.split("\r\n")
        .flat_map(|line| line.split(['\r', '\n']))
        .map(str::to_string)
        .collect()
}

/// Monaco's `paste` helper: an `isBlock` payload whose rows are the text's
/// lines, unless overridden.
struct Payload {
    multicursor_text: Option<Vec<String>>,
    is_block: bool,
}

impl Payload {
    fn block(text: &str) -> Self {
        Self {
            multicursor_text: Some(split_lines(text)),
            is_block: true,
        }
    }
}

struct Scenario<'a> {
    text: &'a [&'a str],
    selections: &'a [Selection],
    expected: &'a [&'a str],
    pasted_text: &'a str,
    tab_size: u32,
    multi_cursor_paste: MultiCursorPaste,
    overtype: bool,
    payload: Option<Payload>,
}

impl Default for Scenario<'_> {
    fn default() -> Self {
        Self {
            text: &[],
            selections: &[],
            expected: &[],
            pasted_text: BLOCK,
            tab_size: 4,
            multi_cursor_paste: MultiCursorPaste::Spread,
            overtype: false,
            payload: None,
        }
    }
}

fn editor(text: &[&str], tab_size: u32, multi_cursor_paste: MultiCursorPaste) -> TestEditor {
    TestEditor::lines_with_config(
        text,
        CursorConfig {
            tab_size,
            multi_cursor_paste,
            ..CursorConfig::default()
        },
    )
}

fn paste(editor: &mut TestEditor, text: &str, payload: Option<Payload>) {
    let payload = payload.unwrap_or_else(|| Payload::block(text));
    editor.paste_block(
        text,
        false,
        payload.multicursor_text.as_deref(),
        payload.is_block,
    );
}

fn assert_paste(scenario: Scenario) {
    let mut editor = editor(
        scenario.text,
        scenario.tab_size,
        scenario.multi_cursor_paste,
    );
    editor.config.overtype = scenario.overtype;
    editor.set_selections(scenario.selections);
    paste(&mut editor, scenario.pasted_text, scenario.payload);
    assert_eq!(editor.value(), scenario.expected.join("\n"));
}

const LEFT_RIGHT: &[&str] = &["left--right", "left--right", "left--right"];

#[test]
fn inserts_successive_rows_from_a_single_cursor() {
    assert_paste(Scenario {
        text: LEFT_RIGHT,
        selections: &[sel(1, 5, 1, 5)],
        expected: &["leftA --right", "leftBC--right", "leftD --right"],
        ..Default::default()
    });
}

#[test]
fn replaces_a_single_line_selection_with_the_first_row() {
    assert_paste(Scenario {
        text: LEFT_RIGHT,
        selections: &[sel(1, 5, 1, 7)],
        expected: &["leftA right", "leftBC--right", "leftD --right"],
        ..Default::default()
    });
}

#[test]
fn uses_the_start_of_a_reversed_selection() {
    assert_paste(Scenario {
        text: LEFT_RIGHT,
        selections: &[sel(1, 7, 1, 5)],
        expected: &["leftA right", "leftBC--right", "leftD --right"],
        ..Default::default()
    });
}

#[test]
fn replaces_an_equal_height_continuous_selection_with_the_first_row() {
    assert_paste(Scenario {
        text: LEFT_RIGHT,
        selections: &[sel(1, 5, 3, 7)],
        expected: &["leftA right", "    BC", "    D "],
        ..Default::default()
    });
}

#[test]
fn replaces_a_reversed_equal_height_continuous_selection() {
    assert_paste(Scenario {
        text: LEFT_RIGHT,
        selections: &[sel(3, 7, 1, 5)],
        expected: &["leftA right", "    BC", "    D "],
        ..Default::default()
    });
}

#[test]
fn replaces_a_full_line_selection_including_its_final_newline() {
    assert_paste(Scenario {
        text: &["first", "second", "third", "unselected"],
        selections: &[sel(1, 1, 4, 1)],
        expected: &["A unselected", "BC", "D "],
        ..Default::default()
    });
}

#[test]
fn preserves_the_suffix_when_a_reversed_selection_ends_at_column_one() {
    assert_paste(Scenario {
        text: &["left--right", "second", "third", "unselected"],
        selections: &[sel(4, 1, 1, 5)],
        expected: &["leftA unselected", "    BC", "    D "],
        ..Default::default()
    });
}

#[test]
fn replaces_a_shorter_continuous_selection_with_the_first_row() {
    assert_paste(Scenario {
        text: &["left--right", "left--right", "last"],
        selections: &[sel(1, 5, 2, 7)],
        expected: &["leftA right", "lastBC", "    D "],
        ..Default::default()
    });
}

#[test]
fn replaces_a_taller_continuous_selection_with_the_first_row() {
    assert_paste(Scenario {
        text: &["left--right", "middle", "middle", "left--right"],
        selections: &[sel(1, 5, 4, 7)],
        expected: &["leftA right", "    BC", "    D "],
        ..Default::default()
    });
}

#[test]
fn pads_short_and_empty_lines_to_the_block_column() {
    assert_paste(Scenario {
        text: &["left--right", "x", ""],
        selections: &[sel(1, 5, 1, 5)],
        expected: &["leftA --right", "x   BC", "    D "],
        ..Default::default()
    });
}

#[test]
fn appends_missing_rows_without_moving_the_existing_suffix() {
    assert_paste(Scenario {
        text: &["left--right"],
        selections: &[sel(1, 5, 1, 5)],
        expected: &["leftA --right", "    BC", "    D "],
        ..Default::default()
    });
}

#[test]
fn appends_rows_when_the_cursor_is_at_the_end_of_the_file() {
    assert_paste(Scenario {
        text: &["left"],
        selections: &[sel(1, 5, 1, 5)],
        expected: &["leftA ", "    BC", "    D "],
        ..Default::default()
    });
}

#[test]
fn pastes_into_an_empty_document() {
    assert_paste(Scenario {
        text: &[""],
        selections: &[sel(1, 1, 1, 1)],
        expected: &["A ", "BC", "D "],
        ..Default::default()
    });
}

#[test]
fn preserves_empty_and_trailing_copied_rows() {
    assert_paste(Scenario {
        text: &["left--right", "left--right", "left--right", "left--right"],
        selections: &[sel(1, 5, 1, 5)],
        pasted_text: "A\n\nD\n",
        expected: &[
            "leftA--right",
            "left --right",
            "leftD--right",
            "left --right",
        ],
        ..Default::default()
    });
}

#[test]
fn pads_short_lines_for_empty_and_trailing_copied_rows() {
    assert_paste(Scenario {
        text: &["left", "x", "", "y"],
        selections: &[sel(1, 5, 1, 5)],
        pasted_text: "A\n\nD\n",
        expected: &["leftA", "x    ", "    D", "y    "],
        ..Default::default()
    });
}

#[test]
fn appends_empty_and_trailing_copied_rows() {
    assert_paste(Scenario {
        text: &["left"],
        selections: &[sel(1, 5, 1, 5)],
        pasted_text: "A\n\nD\n",
        expected: &["leftA", "     ", "    D", "     "],
        ..Default::default()
    });
}

#[test]
fn aligns_using_visible_columns_across_tabs_and_spaces() {
    assert_paste(Scenario {
        text: &["\tleft", "    left", "  \tleft"],
        selections: &[sel(1, 2, 1, 2)],
        expected: &["\tA left", "    BCleft", "  \tD left"],
        ..Default::default()
    });
}

#[test]
fn pads_short_lines_using_the_configured_tab_size() {
    assert_paste(Scenario {
        text: &["\t--left", "\t", ""],
        selections: &[sel(1, 4, 1, 4)],
        tab_size: 8,
        expected: &["\t--A left", "\t  BC", "          D "],
        ..Default::default()
    });
}

#[test]
fn pads_using_visible_widths_of_graphemes_and_full_width_characters() {
    assert_paste(Scenario {
        text: &["left--right", "e\u{0301}", "\u{4E2D}", "\u{1F600}"],
        selections: &[sel(1, 5, 1, 5)],
        pasted_text: "A\nB\nC\nD",
        expected: &[
            "leftA--right",
            "e\u{0301}   B",
            "\u{4E2D}  C",
            "\u{1F600}  D",
        ],
        ..Default::default()
    });
}

#[test]
fn uses_grapheme_boundaries_and_full_width_character_columns() {
    assert_paste(Scenario {
        text: &["\u{1F600}left", "e\u{0301}xleft", "\u{4E2D}left"],
        selections: &[sel(1, 3, 1, 3)],
        expected: &["\u{1F600}A left", "e\u{0301}xBCleft", "\u{4E2D}D left"],
        ..Default::default()
    });
}

#[test]
fn pads_copied_rows_to_the_same_visible_width() {
    assert_paste(Scenario {
        text: &["left--right", "left--right", "left--right", "left--right"],
        selections: &[sel(1, 5, 1, 5)],
        pasted_text: "\tX\n\u{4E2D}\n\u{1F600}\ne\u{0301}",
        expected: &[
            "left\tX--right",
            "left\u{4E2D}   --right",
            "left\u{1F600}   --right",
            "lefte\u{0301}    --right",
        ],
        ..Default::default()
    });
}

#[test]
fn distributes_matching_cursors_in_document_order() {
    assert_paste(Scenario {
        text: &["xx", "yy", "zz"],
        selections: &[sel(3, 2, 3, 2), sel(1, 2, 1, 2), sel(2, 2, 2, 2)],
        expected: &["xAx", "yBCy", "zDz"],
        ..Default::default()
    });
}

#[test]
fn distributes_matching_non_empty_selections() {
    assert_paste(Scenario {
        text: &["[one]", "[two]", "[three]"],
        selections: &[sel(1, 2, 1, 5), sel(2, 2, 2, 5), sel(3, 2, 3, 7)],
        expected: &["[A]", "[BC]", "[D]"],
        ..Default::default()
    });
}

#[test]
fn repeats_the_whole_text_when_multiple_cursor_counts_differ() {
    assert_paste(Scenario {
        text: &["xx", "yy", "zz"],
        selections: &[sel(1, 2, 1, 2), sel(3, 2, 3, 2)],
        expected: &["xA", "BC", "Dx", "yy", "zA", "BC", "Dz"],
        ..Default::default()
    });
}

#[test]
fn uses_normal_replacement_for_multiple_destinations_including_a_multiline_selection() {
    assert_paste(Scenario {
        text: &["xx", "yy", "zz", "[one]", "[two]"],
        selections: &[sel(1, 2, 1, 2), sel(4, 2, 5, 5)],
        expected: &["xA", "BC", "Dx", "yy", "zz", "[A", "BC", "D]"],
        ..Default::default()
    });
}

#[test]
fn retains_ordinary_trailing_newline_handling_when_spreading_to_multiple_cursors() {
    assert_paste(Scenario {
        text: &["xx", "yy"],
        selections: &[sel(1, 2, 1, 2), sel(2, 2, 2, 2)],
        pasted_text: "A\nB\n",
        expected: &["xAx", "yBy"],
        ..Default::default()
    });
}

#[test]
fn block_mode_respects_multi_cursor_paste_full_for_multiple_destinations_without_row_metadata() {
    assert_paste(Scenario {
        text: &["xx", "yy", "zz"],
        selections: &[sel(1, 2, 1, 2), sel(2, 2, 2, 2), sel(3, 2, 3, 2)],
        multi_cursor_paste: MultiCursorPaste::Full,
        payload: Some(Payload {
            multicursor_text: None,
            is_block: true,
        }),
        expected: &["xA", "BC", "Dx", "yA", "BC", "Dy", "zA", "BC", "Dz"],
        ..Default::default()
    });
}

#[test]
fn full_mode_retains_single_cursor_multiline_pasting() {
    assert_paste(Scenario {
        text: &["left--right", "last"],
        selections: &[sel(1, 5, 1, 5)],
        multi_cursor_paste: MultiCursorPaste::Full,
        expected: &["leftA", "BC", "D--right", "last"],
        ..Default::default()
    });
}

#[test]
fn full_mode_retains_existing_multi_cursor_metadata_distribution() {
    assert_paste(Scenario {
        text: &["xx", "yy", "zz"],
        selections: &[sel(1, 2, 1, 2), sel(2, 2, 2, 2), sel(3, 2, 3, 2)],
        multi_cursor_paste: MultiCursorPaste::Full,
        expected: &["xAx", "yBCy", "zDz"],
        ..Default::default()
    });
}

#[test]
fn ordinary_multiline_copies_spread_into_a_rectangle_at_a_single_cursor() {
    assert_paste(Scenario {
        text: &["left--right", "last"],
        selections: &[sel(1, 5, 1, 5)],
        expected: &["leftA --right", "lastBC", "    D "],
        ..Default::default()
    });
}

#[test]
fn missing_block_metadata_still_spreads_multiline_text_at_a_single_cursor() {
    assert_paste(Scenario {
        text: &["left--right", "last"],
        selections: &[sel(1, 5, 1, 5)],
        payload: Some(Payload {
            multicursor_text: None,
            is_block: true,
        }),
        expected: &["leftA --right", "lastBC", "    D "],
        ..Default::default()
    });
}

#[test]
fn preserves_ordinary_spread_pasting_without_metadata() {
    assert_paste(Scenario {
        text: &["xx", "yy", "zz"],
        selections: &[sel(1, 2, 1, 2), sel(2, 2, 2, 2), sel(3, 2, 3, 2)],
        payload: Some(Payload {
            multicursor_text: None,
            is_block: false,
        }),
        expected: &["xAx", "yBCy", "zDz"],
        ..Default::default()
    });
}

#[test]
fn preserves_ordinary_full_pasting_without_metadata() {
    assert_paste(Scenario {
        text: &["xx", "yy"],
        selections: &[sel(1, 2, 1, 2), sel(2, 2, 2, 2)],
        pasted_text: "A\nB",
        multi_cursor_paste: MultiCursorPaste::Full,
        payload: Some(Payload {
            multicursor_text: None,
            is_block: false,
        }),
        expected: &["xA", "Bx", "yA", "By"],
        ..Default::default()
    });
}

#[test]
fn preserves_multi_cursor_distribution_through_undo_and_redo_in_block_mode() {
    let text = ["xx", "yy", "zz"];
    let mut editor = editor(&text, 4, MultiCursorPaste::Spread);
    let selections = [sel(1, 2, 1, 2), sel(2, 2, 2, 2), sel(3, 2, 3, 2)];
    editor.set_selections(&selections);
    paste(&mut editor, BLOCK, None);
    let expected_text = ["xAx", "yBCy", "zDz"].join("\n");
    let expected_selections = [sel(1, 3, 1, 3), sel(2, 4, 2, 4), sel(3, 3, 3, 3)];
    assert_eq!(editor.value(), expected_text);
    editor.assert_selections(&expected_selections);
    editor.undo();
    assert_eq!(editor.value(), text.join("\n"));
    editor.assert_selections(&selections);
    editor.redo();
    assert_eq!(editor.value(), expected_text);
    editor.assert_selections(&expected_selections);
}

#[test]
fn replaces_a_multiline_selection_and_aligns_later_rows_through_undo_and_redo() {
    let text = ["\tfirst", "middle", "last--suffix", "x", ""];
    let mut editor = editor(&text, 4, MultiCursorPaste::Spread);
    let selection = sel(3, 7, 1, 2);
    editor.set_selection(selection);
    paste(&mut editor, "\nBC\n", None);
    let expected_text = ["\t  suffix", "x   BC", "      "].join("\n");
    assert_eq!(editor.value(), expected_text);
    editor.assert_selections(&[sel(3, 7, 3, 7)]);
    editor.undo();
    assert_eq!(editor.value(), text.join("\n"));
    editor.assert_selections(&[selection]);
    editor.redo();
    assert_eq!(editor.value(), expected_text);
    editor.assert_selections(&[sel(3, 7, 3, 7)]);
}

#[test]
fn leaves_the_cursor_after_rectangular_padding_on_an_empty_trailing_pasted_row() {
    let mut editor = editor(&["left", "left", "left"], 4, MultiCursorPaste::Spread);
    editor.set_position(1, 5);
    paste(&mut editor, "A\nBC\n", None);
    assert_eq!(editor.value(), ["leftA ", "leftBC", "left  "].join("\n"));
    editor.assert_selections(&[sel(3, 7, 3, 7)]);
}

#[test]
fn leaves_the_cursor_after_padding_on_an_empty_trailing_pasted_row() {
    let mut editor = editor(&["left", "x", ""], 4, MultiCursorPaste::Spread);
    editor.set_position(1, 5);
    paste(&mut editor, "A\nBC\n", None);
    assert_eq!(editor.value(), ["leftA ", "x   BC", "      "].join("\n"));
    editor.assert_selections(&[sel(3, 7, 3, 7)]);
}

#[test]
fn leaves_the_cursor_after_padding_on_an_appended_empty_row() {
    let mut editor = editor(&["left"], 4, MultiCursorPaste::Spread);
    editor.set_position(1, 5);
    paste(&mut editor, "A\nBC\n", None);
    assert_eq!(editor.value(), ["leftA ", "    BC", "      "].join("\n"));
    editor.assert_selections(&[sel(3, 7, 3, 7)]);
}

#[test]
fn undo_and_redo_preserve_text_and_the_original_single_selection() {
    let mut editor = editor(&["left--right", "x"], 4, MultiCursorPaste::Spread);
    let selection = sel(1, 7, 1, 5);
    editor.set_selection(selection);
    paste(&mut editor, BLOCK, None);
    let expected_text = ["leftA right", "x   BC", "    D "].join("\n");
    assert_eq!(editor.value(), expected_text);
    editor.assert_selections(&[sel(3, 7, 3, 7)]);
    editor.undo();
    assert_eq!(editor.value(), ["left--right", "x"].join("\n"));
    editor.assert_selections(&[selection]);
    editor.redo();
    assert_eq!(editor.value(), expected_text);
    editor.assert_selections(&[sel(3, 7, 3, 7)]);
}

#[test]
fn respects_overtype_and_pads_short_rows() {
    assert_paste(Scenario {
        text: &["left--right", "left--right", "x", ""],
        selections: &[sel(1, 5, 1, 5)],
        pasted_text: "A\nBC\nD\nE",
        overtype: true,
        expected: &["leftA right", "leftBCright", "x   D ", "    E "],
        ..Default::default()
    });
}

#[test]
fn copies_use_the_line_ending() {
    // Monaco `getPlainTextToCopy`/`getDataToCopy`: the model's EOL within
    // and between pieces and after whole lines, or CRLF within pieces when
    // forced (Windows).
    use text::LineEnding;
    let mut editor = TestEditor::lines(&["one", "two", "three"]);
    editor.set_selection(sel(1, 2, 2, 3));
    assert_eq!(editor.copy_with_eol(LineEnding::Crlf, false).0, "ne\r\ntw");
    assert_eq!(editor.copy_with_eol(LineEnding::Lf, true).0, "ne\r\ntw");
    assert_eq!(editor.copy().0, "ne\ntw");
    editor.set_position(1, 1);
    assert_eq!(
        editor.copy_with_eol(LineEnding::Crlf, false),
        ("one\r\n".to_string(), true, Vec::new())
    );
    editor.set_selections(&[sel(1, 1, 1, 1), sel(3, 1, 3, 3)]);
    assert_eq!(
        editor.copy_with_eol(LineEnding::Crlf, false),
        (
            "one\r\n\r\nth".to_string(),
            false,
            vec!["one\r\n".to_string(), "th".to_string()]
        )
    );
    // Forced CRLF pieces are still joined with the model's EOL.
    assert_eq!(
        editor.copy_with_eol(LineEnding::Lf, true).0,
        "one\r\n\nth".to_string()
    );
}
