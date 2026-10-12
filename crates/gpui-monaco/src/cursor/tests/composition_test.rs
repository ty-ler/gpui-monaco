//! The IME composition tests of Monaco's
//! `src/vs/editor/test/browser/controller/cursor.test.ts` (MIT), suite
//! `Editor Controller`: typing during a composition skips the interceptors,
//! and ending it runs `compositionEndWithInterceptors`.

use super::harness::{TestEditor, sel};
use super::language_configs::auto_closing_language;

fn using_cursor(text: &[&str]) -> TestEditor {
    TestEditor::lines(text).with_language(auto_closing_language())
}

/// Monaco `issue #15825: accents on mac US intl keyboard`.
#[test]
fn issue_15825_accents_on_mac_us_intl_keyboard() {
    let mut ed = using_cursor(&[]);
    ed.assert_cursor(1, 1);
    // Typing ` + e on the mac US intl kb layout.
    ed.start_composition();
    ed.type_keyboard("`");
    ed.composition_type("è", 1, 0, 0);
    ed.end_composition();
    assert_eq!(ed.value(), "è");
}

/// Monaco `issue #90016: allow accents on mac US intl keyboard to surround
/// selection`.
#[test]
fn issue_90016_allow_accents_on_mac_us_intl_keyboard_to_surround_selection() {
    let mut ed = using_cursor(&["test"]);
    ed.set_selection(sel(1, 1, 1, 5));
    // Typing ` + e on the mac US intl kb layout.
    ed.start_composition();
    ed.type_keyboard("'");
    ed.composition_type("'", 1, 0, 0);
    ed.composition_type("'", 1, 0, 0);
    ed.end_composition();
    assert_eq!(ed.value(), "'test'");
}

/// Monaco `issue #2773: Accents (´`¨^, others?) are inserted in the wrong
/// position (Mac)`.
#[test]
fn issue_2773_accents_are_inserted_in_the_wrong_position() {
    let mut ed = using_cursor(&["hello", "world"]);
    ed.assert_cursor(1, 1);
    // Typing ` and pressing shift+down on the mac US intl kb layout. Here
    // we're just replaying what the cursor gets.
    ed.start_composition();
    ed.type_keyboard("`");
    ed.move_down(true);
    ed.composition_type("`", 1, 0, 0);
    ed.composition_type("`", 1, 0, 0);
    ed.end_composition();
    assert_eq!(ed.value(), "`hello\nworld");
    ed.assert_selection(sel(1, 2, 2, 2));
}

/// Types `'` + space on the mac US intl kb layout.
fn compose_quote(ed: &mut TestEditor) {
    ed.start_composition();
    ed.type_keyboard("'");
    ed.composition_type("'", 1, 0, 0);
    ed.end_composition();
}

/// Monaco `issue #26820: auto close quotes when not used as accents`.
#[test]
fn issue_26820_auto_close_quotes_when_not_used_as_accents() {
    let mut ed = using_cursor(&[""]);
    ed.assert_cursor(1, 1);

    // Typing ' + space.
    compose_quote(&mut ed);
    assert_eq!(ed.value(), "''");

    // Typing one more ' + space.
    compose_quote(&mut ed);
    assert_eq!(ed.value(), "''");

    // Typing ' as a closing tag.
    ed.set_value("'abc");
    ed.set_selection(sel(1, 5, 1, 5));
    compose_quote(&mut ed);
    assert_eq!(ed.value(), "'abc'");

    // Quotes before the newly added character are all paired.
    ed.set_value("'abc'def ");
    ed.set_selection(sel(1, 10, 1, 10));
    compose_quote(&mut ed);
    assert_eq!(ed.value(), "'abc'def ''");

    // No auto closing if there is non-whitespace character after the cursor.
    ed.set_value("abc");
    ed.set_selection(sel(1, 1, 1, 1));
    compose_quote(&mut ed);

    // No auto closing if it's after a word.
    ed.set_value("abc");
    ed.set_selection(sel(1, 4, 1, 4));
    compose_quote(&mut ed);
    assert_eq!(ed.value(), "abc'");
}

/// Monaco `issue #144690: Quotes do not overtype when using US Intl PC
/// keyboard layout`.
#[test]
fn issue_144690_quotes_do_not_overtype_when_using_us_intl_pc_keyboard_layout() {
    let mut ed = using_cursor(&[""]);
    ed.assert_cursor(1, 1);

    // Pressing ' + ' + ;
    ed.start_composition();
    ed.type_keyboard("'");
    ed.composition_type("'", 1, 0, 0);
    ed.composition_type("'", 1, 0, 0);
    ed.end_composition();
    ed.start_composition();
    ed.type_keyboard("'");
    ed.composition_type("';", 1, 0, 0);
    ed.composition_type("';", 2, 0, 0);
    ed.end_composition();

    assert_eq!(ed.value(), "'';");
}

/// Monaco `issue #144693: Typing a quote using US Intl PC keyboard layout
/// always surrounds words`.
#[test]
fn issue_144693_typing_a_quote_using_us_intl_pc_keyboard_layout_always_surrounds_words() {
    let mut ed = using_cursor(&["const hello = 3;"]);
    ed.set_selection(sel(1, 7, 1, 12));

    // Pressing ' + e
    ed.start_composition();
    ed.type_keyboard("'");
    ed.composition_type("é", 1, 0, 0);
    ed.composition_type("é", 1, 0, 0);
    ed.end_composition();

    assert_eq!(ed.value(), "const é = 3;");
}

/// Monaco `issue #82701: auto close does not execute when IME is canceled via
/// backspace`.
#[test]
fn issue_82701_auto_close_does_not_execute_when_ime_is_canceled_via_backspace() {
    let mut ed = using_cursor(&["{}"]);
    ed.set_selection(sel(1, 2, 1, 2));

    // Typing a + backspace
    ed.start_composition();
    ed.type_keyboard("a");
    ed.composition_type("", 1, 0, 0);
    ed.end_composition();
    assert_eq!(ed.value(), "{}");
}

/// Typing an opening bracket during a composition doesn't auto-close it
/// (Monaco passes `isDoingComposition` to the interceptors); the
/// composition's end does, once it's known to have typed one character.
#[test]
fn composing_skips_the_interceptors_until_the_composition_ends() {
    let mut ed = using_cursor(&[""]);
    ed.start_composition();
    ed.type_keyboard("(");
    assert_eq!(ed.value(), "(");
    ed.end_composition();
    assert_eq!(ed.value(), "()");
    ed.assert_cursor(1, 2);
    // And the closing one it added is auto-closed: typing it types over it.
    ed.type_keyboard(")");
    assert_eq!(ed.value(), "()");
    ed.assert_cursor(1, 3);
}
