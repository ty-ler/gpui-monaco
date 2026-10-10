//! Drop and paste edit providers (Monaco `contrib/dropOrPasteInto`).

use std::path::PathBuf;

use gpui::{
    App, ClipboardItem, Entity, Focusable, SharedString, Task, TestAppContext, VisualTestContext,
};
use gpui_monaco::CodeEditor;
use gpui_monaco::actions::{ChangePasteType, Paste};
use gpui_monaco::providers::{
    DataTransfer, DocumentDropOrPasteEdit, DocumentPasteEditProvider, DocumentPasteTriggerKind,
    EditInsertText, LanguageSelector, ProviderContext, register_document_paste_edit_provider,
};
use gpui_monaco::text::PointUtf16;

fn setup<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| CodeEditor::with_text(&text, window, cx));
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    paint(cx);
    (editor, cx)
}

fn paint(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn text(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |e, cx| e.model().read(cx).text())
}

/// Pastes text upper-cased.
struct Shout;

impl DocumentPasteEditProvider for Shout {
    fn paste_mime_types(&self) -> Vec<SharedString> {
        vec!["text/plain".into()]
    }

    fn provide_document_paste_edits(
        &self,
        _: &ProviderContext,
        _: &[std::ops::Range<PointUtf16>],
        data: &DataTransfer,
        _: DocumentPasteTriggerKind,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<DocumentDropOrPasteEdit>>> {
        let text = data.text("text/plain").unwrap_or_default().to_uppercase();
        Task::ready(Ok(vec![DocumentDropOrPasteEdit {
            title: "Shout".into(),
            kind: Some("text.shout".into()),
            insert_text: EditInsertText::Text(text),
            ..Default::default()
        }]))
    }
}

#[gpui::test]
fn dropping_files_inserts_their_paths(cx: &mut TestAppContext) {
    let (editor, cx) = setup("ab", cx);
    editor.update(cx, |e, cx| {
        e.drop_files(&[PathBuf::from("/tmp/x y.txt")], PointUtf16::new(0, 1), cx)
    });
    paint(cx);
    assert_eq!(text(&editor, cx), "a/tmp/x y.txtb");
    // One edit: no options button.
    assert!(editor.read_with(cx, |e, _| !e.is_post_edit_widget_visible()));
}

#[gpui::test]
fn providers_decide_how_to_paste(cx: &mut TestAppContext) {
    let (editor, cx) = setup("", cx);
    // Without providers of its own, the editor pastes normally.
    cx.write_to_clipboard(ClipboardItem::new_string("hi".into()));
    cx.dispatch_action(Paste);
    paint(cx);
    assert_eq!(text(&editor, cx), "hi");

    cx.update(|_, cx| {
        register_document_paste_edit_provider(cx, LanguageSelector::All, Shout);
    });
    cx.dispatch_action(Paste);
    paint(cx);
    assert_eq!(text(&editor, cx), "hiHI");
    assert!(editor.read_with(cx, |e, _| e.is_post_edit_widget_visible()));

    // Switch to plain text.
    cx.dispatch_action(ChangePasteType);
    paint(cx);
    let rows = editor.read_with(cx, |e, cx| e.quick_access_rows_for_tests(cx));
    assert_eq!(rows, vec!["> Shout ✓", "Insert Plain Text"]);
    cx.simulate_keystrokes("down enter");
    paint(cx);
    assert_eq!(text(&editor, cx), "hihi");
}

#[gpui::test]
fn paste_as_text(cx: &mut TestAppContext) {
    let (editor, cx) = setup("", cx);
    cx.update(|_, cx| {
        register_document_paste_edit_provider(cx, LanguageSelector::All, Shout);
    });
    cx.write_to_clipboard(ClipboardItem::new_string("hi".into()));
    editor.update_in(cx, |e, window, cx| {
        e.paste_as(Some("text.plain".into()), window, cx)
    });
    paint(cx);
    assert_eq!(text(&editor, cx), "hi");
}
