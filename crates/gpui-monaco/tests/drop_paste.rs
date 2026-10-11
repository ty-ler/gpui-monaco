//! Drop and paste edit providers (Monaco `contrib/dropOrPasteInto`).

use std::path::PathBuf;

use gpui::{
    App, ClipboardItem, Entity, Focusable, SharedString, Task, TestAppContext, VisualTestContext,
};
use gpui_monaco::CodeEditor;
use gpui_monaco::actions::{ChangePasteType, Paste};
use gpui_monaco::providers::{
    DataTransfer, DocumentDropOrPasteEdit, DocumentPasteContext, DocumentPasteEditProvider,
    DocumentPasteTriggerKind, EditInsertText, LanguageSelector, ProviderContext, WorkspaceEdit,
    register_document_paste_edit_provider,
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

#[gpui::test]
fn pasting_copied_files_inserts_their_paths(cx: &mut TestAppContext) {
    let (editor, cx) = setup("", cx);
    let files = gpui::ExternalPaths(vec![PathBuf::from("/tmp/a.txt")].into());
    cx.write_to_clipboard(ClipboardItem::from(gpui::ClipboardEntry::ExternalPaths(
        files,
    )));
    cx.dispatch_action(Paste);
    paint(cx);
    assert_eq!(text(&editor, cx), "/tmp/a.txt");
}

#[gpui::test]
fn middle_click(cx: &mut TestAppContext) {
    use gpui::{Modifiers, MouseButton, point, px};
    let (editor, cx) = setup("abc\nabc", cx);
    let at = editor.read_with(cx, |e, _| {
        e.scrolled_visible_position(PointUtf16::new(1, 2)).unwrap()
    });
    cx.simulate_mouse_down(
        point(at.x + px(1.), at.y + px(5.)),
        MouseButton::Middle,
        Modifiers::default(),
    );
    // A click with the selection clipboard (Linux), else the start of a
    // column selection: either way the cursor goes there.
    let selections = editor.read_with(cx, |e, _| e.selections());
    assert_eq!(selections.len(), 1);
    assert_eq!(selections[0].head, PointUtf16::new(1, 2));
}

#[gpui::test]
fn auto_indent_on_paste(cx: &mut TestAppContext) {
    use gpui::AppContext as _;
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
    });
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| {
            gpui_monaco::TextModel::new("function f() {\n\n}").with_language("typescript")
        });
        let mut editor = CodeEditor::new(model, window, cx);
        let mut options = editor.options().clone();
        options.auto_indent_on_paste = true;
        editor.update_options(options, cx);
        editor
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    paint(cx);
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(
                1, 0,
            ))],
            cx,
        )
    });
    cx.write_to_clipboard(ClipboardItem::new_string("let x = 1;\nlet y = 2;".into()));
    cx.dispatch_action(Paste);
    paint(cx);
    assert_eq!(
        text(&editor, cx),
        "function f() {\n    let x = 1;\n    let y = 2;\n}"
    );
}

/// Fills in its edits only when one is applied, and records the paste
/// context it's asked with.
struct Lazy(std::rc::Rc<std::cell::RefCell<Vec<Option<String>>>>);

impl DocumentPasteEditProvider for Lazy {
    fn paste_mime_types(&self) -> Vec<SharedString> {
        vec!["text/plain".into()]
    }

    fn provided_paste_edit_kinds(&self) -> Vec<SharedString> {
        vec!["text.lazy".into()]
    }

    fn provide_document_paste_edits(
        &self,
        _: &ProviderContext,
        _: &[std::ops::Range<PointUtf16>],
        _: &DataTransfer,
        _: DocumentPasteTriggerKind,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<DocumentDropOrPasteEdit>>> {
        unreachable!("asked with the context")
    }

    fn provide_document_paste_edits_with_context(
        &self,
        _: &ProviderContext,
        _: &[std::ops::Range<PointUtf16>],
        _: &DataTransfer,
        paste_context: &DocumentPasteContext,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<DocumentDropOrPasteEdit>>> {
        self.0
            .borrow_mut()
            .push(paste_context.only.as_ref().map(|only| only.to_string()));
        Task::ready(Ok(vec![DocumentDropOrPasteEdit {
            title: "Lazy".into(),
            kind: Some("text.lazy".into()),
            insert_text: EditInsertText::Text("unresolved".into()),
            ..Default::default()
        }]))
    }

    fn resolve_document_paste_edit(
        &self,
        edit: &DocumentDropOrPasteEdit,
        _: &mut App,
    ) -> Option<Task<anyhow::Result<DocumentDropOrPasteEdit>>> {
        Some(Task::ready(Ok(DocumentDropOrPasteEdit {
            insert_text: EditInsertText::Text("resolved".into()),
            // Monaco `additionalEdit`: a workspace edit applied after it.
            additional_edit: Some(WorkspaceEdit {
                edits: vec![(
                    None,
                    vec![(
                        PointUtf16::new(0, 0)..PointUtf16::new(0, 0),
                        "// pasted\n".into(),
                    )],
                )],
                ..Default::default()
            }),
            ..edit.clone()
        })))
    }
}

/// Monaco `resolveDocumentPasteEdit` and `DocumentPasteContext.only`: the
/// edit applied is resolved first, and "Paste As..." with a kind says
/// which it wants.
#[gpui::test]
fn paste_edits_are_resolved_before_applying(cx: &mut TestAppContext) {
    let (editor, cx) = setup("", cx);
    let asked = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let provider = Lazy(asked.clone());
    cx.update(|_, cx| {
        register_document_paste_edit_provider(cx, LanguageSelector::All, provider);
    });
    cx.write_to_clipboard(ClipboardItem::new_string("hi".into()));
    cx.dispatch_action(Paste);
    paint(cx);
    assert_eq!(text(&editor, cx), "// pasted\nresolved");
    editor.update(cx, |e, cx| {
        e.model().update(cx, |m, cx| m.set_value("", cx))
    });
    editor.update_in(cx, |e, window, cx| {
        e.paste_as(Some("text.lazy".into()), window, cx)
    });
    paint(cx);
    assert_eq!(text(&editor, cx), "// pasted\nresolved");
    assert_eq!(*asked.borrow(), [None, Some("text.lazy".to_string())]);
}
