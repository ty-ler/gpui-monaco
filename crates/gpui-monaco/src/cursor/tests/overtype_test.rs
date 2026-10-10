//! Ported from the "Overtype Mode" suite of Monaco's
//! `src/vs/editor/test/browser/controller/cursor.test.ts` (MIT). The
//! composition case is covered by the editor tests, since compositions are
//! committed by the input handler.

use super::harness::{TestEditor, sel};
use crate::cursor::CursorConfig;

fn overtype_editor() -> TestEditor {
    TestEditor::lines_with_config(
        &["123456789", "123456789"],
        CursorConfig {
            insert_spaces: false,
            overtype: true,
            ..CursorConfig::default()
        },
    )
}

/// Monaco: simple type
#[test]
fn simple_type() {
    let mut editor = overtype_editor();
    editor.set_selection(sel(1, 3, 1, 3));
    editor.type_keyboard("a");
    assert_eq!(editor.value(), "12a456789\n123456789");
    editor.set_selection(sel(1, 9, 1, 9));
    editor.type_keyboard("bbb");
    assert_eq!(editor.value(), "12a45678bbb\n123456789");
}

/// Monaco: multi-line selection type
#[test]
fn multi_line_selection_type() {
    let mut editor = overtype_editor();
    editor.set_selection(sel(1, 5, 2, 3));
    editor.type_raw("cc");
    assert_eq!(editor.value(), "1234cc456789");
}

/// Monaco: simple paste
#[test]
fn simple_paste() {
    let mut editor = overtype_editor();
    editor.set_selection(sel(1, 5, 1, 5));
    editor.paste("cc", false, None);
    assert_eq!(editor.value(), "1234cc789\n123456789");
    editor.set_selection(sel(1, 5, 1, 5));
    editor.paste("dddddddd", false, None);
    assert_eq!(editor.value(), "1234dddddddd\n123456789");
}

/// Monaco: multi-line selection paste
#[test]
fn multi_line_selection_paste() {
    let mut editor = overtype_editor();
    editor.set_selection(sel(1, 5, 2, 3));
    editor.paste("cc", false, None);
    assert_eq!(editor.value(), "1234cc456789");
}

/// Monaco: paste multi-line text
#[test]
fn paste_multi_line_text() {
    let mut editor = overtype_editor();
    editor.set_selection(sel(1, 5, 1, 5));
    editor.paste("aaaaaaa\nbbbbbbb", false, None);
    assert_eq!(editor.value(), "1234aaaaaaa\nbbbbbbb\n123456789");
}

/// `overtypeOnPaste: false` pastes normally even in overtype mode.
#[test]
fn paste_without_overtype_on_paste() {
    let mut editor = overtype_editor();
    editor.config.overtype_on_paste = false;
    editor.set_selection(sel(1, 5, 1, 5));
    editor.paste("cc", false, None);
    assert_eq!(editor.value(), "1234cc56789\n123456789");
}
