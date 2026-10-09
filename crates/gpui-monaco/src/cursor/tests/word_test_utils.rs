//! Port of Monaco's
//! `src/vs/editor/contrib/wordOperations/test/browser/wordTestUtils.ts` (MIT).

use super::harness::{TestEditor, pos};
use crate::cursor::{CursorConfig, Position};

/// Monaco `deserializePipePositions`: strips `|` markers from `text` and
/// returns the text and the marked positions (UTF-16 columns).
pub fn deserialize_pipe_positions(text: &str) -> (String, Vec<Position>) {
    let mut result_text = String::new();
    let mut line_number = 1;
    let mut char_index = 0;
    let mut positions = Vec::new();
    for chr in text.chars() {
        if chr == '\n' {
            result_text.push(chr);
            line_number += 1;
            char_index = 0;
            continue;
        }
        if chr == '|' {
            positions.push(pos(line_number, char_index + 1));
        } else {
            result_text.push(chr);
            char_index += chr.len_utf16() as u32;
        }
    }
    (result_text, positions)
}

/// Monaco `serializePipePositions`: inserts a `|` at each position.
pub fn serialize_pipe_positions(text: &str, mut positions: Vec<Position>) -> String {
    positions.sort();
    let mut positions = positions.into_iter().peekable();
    let mut result_text = String::new();
    let mut line_number = 1;
    let mut char_index = 0;
    for chr in text.chars() {
        if positions.peek() == Some(&pos(line_number, char_index + 1)) {
            result_text.push('|');
            positions.next();
        }
        result_text.push(chr);
        if chr == '\n' {
            line_number += 1;
            char_index = 0;
        } else {
            char_index += chr.len_utf16() as u32;
        }
    }
    if positions.peek() == Some(&pos(line_number, char_index + 1)) {
        result_text.push('|');
        positions.next();
    }
    assert!(
        positions.next().is_none(),
        "Unexpected left over positions!!!"
    );
    result_text
}

/// Monaco `testRepeatedActionAndExtractPositions`.
pub fn test_repeated_action_and_extract_positions(
    text: &str,
    initial_position: Position,
    config: CursorConfig,
    mut action: impl FnMut(&mut TestEditor),
    record: impl Fn(&TestEditor) -> Position,
    stop_condition: impl Fn(&TestEditor) -> bool,
) -> Vec<Position> {
    let mut actual_stops = Vec::new();
    let mut editor = TestEditor::with_config(text, config);
    let (line, column) = initial_position.to_monaco();
    editor.set_position(line, column);
    loop {
        action(&mut editor);
        actual_stops.push(record(&editor));
        if stop_condition(&editor) {
            break;
        }
        assert!(
            actual_stops.len() <= 1000,
            "Endless loop detected involving position {:?}!",
            editor.position()
        );
    }
    actual_stops
}
