//! Port of Monaco's
//! `src/vs/editor/contrib/wordPartOperations/test/browser/wordPartOperations.test.ts`
//! (MIT). All tests are ported.

use super::harness::{TestEditor, pos};
use super::word_test_utils::{
    deserialize_pipe_positions, serialize_pipe_positions,
    test_repeated_action_and_extract_positions,
};
use crate::cursor::{CursorConfig, MoveCommand, Position, delete_word_part};

fn cursor_word_part_left(ed: &mut TestEditor) {
    ed.move_cursors(MoveCommand::WordPartLeft, false);
}
fn cursor_word_part_right(ed: &mut TestEditor) {
    ed.move_cursors(MoveCommand::WordPartRight, false);
}
fn delete_word_part_left(ed: &mut TestEditor) {
    ed.run(|c, b, cur| delete_word_part(c, b, cur, true));
}
fn delete_word_part_right(ed: &mut TestEditor) {
    ed.run(|c, b, cur| delete_word_part(c, b, cur, false));
}

/// Default separators sans `$-%~`, plus `·`.
fn kebab_config() -> CursorConfig {
    CursorConfig {
        word_separators: "!\"#&'()*+,./:;<=>?@[\\]^`{|}·".to_string(),
        ..CursorConfig::default()
    }
}

#[track_caller]
fn assert_stops(
    expected: &str,
    initial: Position,
    stop: Position,
    config: CursorConfig,
    action: fn(&mut TestEditor),
) {
    let (text, _) = deserialize_pipe_positions(expected);
    let actual = test_repeated_action_and_extract_positions(
        &text,
        initial,
        config,
        action,
        |ed| ed.position(),
        move |ed| ed.position() == stop,
    );
    assert_eq!(serialize_pipe_positions(&text, actual), expected);
}

#[track_caller]
fn assert_delete_left_stops(expected: &str, initial: Position, config: CursorConfig) {
    let (text, _) = deserialize_pipe_positions(expected);
    let actual = test_repeated_action_and_extract_positions(
        &text,
        initial,
        config,
        delete_word_part_left,
        |ed| ed.position(),
        |ed| ed.value().is_empty(),
    );
    assert_eq!(serialize_pipe_positions(&text, actual), expected);
}

#[track_caller]
fn assert_delete_right_stops(expected: &str, config: CursorConfig) {
    let (text, _) = deserialize_pipe_positions(expected);
    let len = text.encode_utf16().count() as u32;
    let actual = test_repeated_action_and_extract_positions(
        &text,
        pos(1, 1),
        config,
        delete_word_part_right,
        |ed| pos(1, len - ed.value().encode_utf16().count() as u32 + 1),
        |ed| ed.value().is_empty(),
    );
    assert_eq!(serialize_pipe_positions(&text, actual), expected);
}

#[test]
fn cursor_word_part_left_basic() {
    let expected = [
        "|start| |line|",
        "|this|Is|A|Camel|Case|Var|  |this_|is_|a_|snake_|case_|var| |THIS_|IS_|CAPS_|SNAKE| |this_|IS|Mixed|Use|",
        "|end| |line",
    ]
    .join("\n");
    assert_stops(
        &expected,
        pos(1000, 1000),
        pos(1, 1),
        CursorConfig::default(),
        cursor_word_part_left,
    );
}

#[test]
fn cursor_word_part_left_issue_53899_whitespace() {
    let expected = "|myvar| |=| |'|demonstration|     |of| |selection| |with| |space|'";
    assert_stops(
        expected,
        pos(1000, 1000),
        pos(1, 1),
        CursorConfig::default(),
        cursor_word_part_left,
    );
}

#[test]
fn cursor_word_part_left_issue_53899_underscores() {
    let expected = "|myvar| |=| |'|demonstration_____|of| |selection| |with| |space|'";
    assert_stops(
        expected,
        pos(1000, 1000),
        pos(1, 1),
        CursorConfig::default(),
        cursor_word_part_left,
    );
}

#[test]
fn cursor_word_part_right_basic() {
    let expected = [
        "start| |line|",
        "|this|Is|A|Camel|Case|Var|  |this|_is|_a|_snake|_case|_var| |THIS|_IS|_CAPS|_SNAKE| |this|_IS|Mixed|Use|",
        "|end| |line|",
    ]
    .join("\n");
    assert_stops(
        &expected,
        pos(1, 1),
        pos(3, 9),
        CursorConfig::default(),
        cursor_word_part_right,
    );
}

#[test]
fn cursor_word_part_right_issue_53899_whitespace() {
    let expected = "myvar| |=| |'|demonstration|     |of| |selection| |with| |space|'|";
    assert_stops(
        expected,
        pos(1, 1),
        pos(1, 52),
        CursorConfig::default(),
        cursor_word_part_right,
    );
}

#[test]
fn cursor_word_part_right_issue_53899_underscores() {
    let expected = "myvar| |=| |'|demonstration|_____of| |selection| |with| |space|'|";
    assert_stops(
        expected,
        pos(1, 1),
        pos(1, 52),
        CursorConfig::default(),
        cursor_word_part_right,
    );
}

#[test]
fn cursor_word_part_right_issue_53899_second_case() {
    let expected = [
        ";| |--| |1|",
        "|;|        |--| |2|",
        "|;|    |#|3|",
        "|;|   |#|4|",
    ]
    .join("\n");
    assert_stops(
        &expected,
        pos(1, 1),
        pos(4, 7),
        CursorConfig::default(),
        cursor_word_part_right,
    );
}

#[test]
fn issue_93239_cursor_word_part_right() {
    let expected = "foo|_bar|";
    assert_stops(
        expected,
        pos(1, 1),
        pos(1, 8),
        CursorConfig::default(),
        cursor_word_part_right,
    );
}

#[test]
fn issue_93239_cursor_word_part_left() {
    let expected = "|foo_|bar";
    assert_stops(
        expected,
        pos(1, 8),
        pos(1, 1),
        CursorConfig::default(),
        cursor_word_part_left,
    );
}

#[test]
fn delete_word_part_left_basic() {
    let expected = "|   |/*| |Just| |some| |text| |a|+=| |3| |+|5|-|3| |*/|  |this|Is|A|Camel|Case|Var|  |this_|is_|a_|snake_|case_|var| |THIS_|IS_|CAPS_|SNAKE| |this_|IS|Mixed|Use";
    assert_delete_left_stops(expected, pos(1, 1000), CursorConfig::default());
}

#[test]
fn delete_word_part_right_basic() {
    let expected = "   |/*| |Just| |some| |text| |a|+=| |3| |+|5|-|3| |*/|  |this|Is|A|Camel|Case|Var|  |this|_is|_a|_snake|_case|_var| |THIS|_IS|_CAPS|_SNAKE| |this|_IS|Mixed|Use|";
    assert_delete_right_stops(expected, CursorConfig::default());
}

#[test]
fn issue_158667_cursor_word_part_left_stops_at_dash() {
    let expected = "|this-|is-|a-|kebab-|case-|var| |THIS-|IS-|CAPS-|KEBAB| |this-|IS|Mixed|Use";
    assert_stops(
        expected,
        pos(1000, 1000),
        pos(1, 1),
        kebab_config(),
        cursor_word_part_left,
    );
}

#[test]
fn issue_158667_cursor_word_part_right_stops_at_dash() {
    let expected = "this|-is|-a|-kebab|-case|-var| |THIS|-IS|-CAPS|-KEBAB| |this|-IS|Mixed|Use|";
    assert_stops(
        expected,
        pos(1, 1),
        pos(1, 60),
        kebab_config(),
        cursor_word_part_right,
    );
}

#[test]
fn issue_158667_delete_word_part_left_stops_at_dash() {
    let expected = "|this-|is-|a-|kebab-|case-|var| |THIS-|IS-|CAPS-|KEBAB| |this-|IS|Mixed|Use";
    assert_delete_left_stops(expected, pos(1000, 1000), kebab_config());
}

#[test]
fn issue_158667_delete_word_part_right_stops_at_dash() {
    let expected = "this|-is|-a|-kebab|-case|-var| |THIS|-IS|-CAPS|-KEBAB| |this|-IS|Mixed|Use|";
    assert_delete_right_stops(expected, kebab_config());
}
