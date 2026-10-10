//! Formatting (Monaco `contrib/format`) with test formatters.

use std::ops::Range;

use gpui::{App, AppContext as _, Entity, Focusable, Task, TestAppContext, VisualTestContext};
use gpui_monaco::cursor::Selection;
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, TextModel};

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

/// Collapses runs of spaces after the indentation and trims line ends.
fn tidy(line: &str) -> String {
    let indent = line.len() - line.trim_start().len();
    let mut out = line[..indent].to_string();
    let mut last_space = false;
    for c in line[indent..].trim_end().chars() {
        if c == ' ' && last_space {
            continue;
        }
        last_space = c == ' ';
        out.push(c);
    }
    out
}

fn line_edits(text: &str, rows: Range<u32>) -> Vec<TextEdit> {
    text.split('\n')
        .enumerate()
        .filter(|(row, _)| rows.contains(&(*row as u32)))
        .filter_map(|(row, line)| {
            let tidy = tidy(line);
            (tidy != line).then(|| {
                let row = row as u32;
                (p(row, 0)..p(row, line.encode_utf16().count() as u32), tidy)
            })
        })
        .collect()
}

struct WholeDocument;
impl DocumentFormattingProvider for WholeDocument {
    fn provide_document_formatting_edits(
        &self,
        context: &ProviderContext,
        _: FormattingOptions,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<TextEdit>>> {
        // A whole-text replacement, like many formatters return.
        let text = context.snapshot.rope().to_string();
        let tidy: Vec<String> = text.split('\n').map(tidy).collect();
        let end = context.snapshot.max_point_utf16();
        Task::ready(Ok(vec![(p(0, 0)..end, tidy.join("\n"))]))
    }
}

struct Lines;
impl DocumentRangeFormattingProvider for Lines {
    fn provide_document_range_formatting_edits(
        &self,
        context: &ProviderContext,
        range: Range<PointUtf16>,
        _: FormattingOptions,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<TextEdit>>> {
        let text = context.snapshot.rope().to_string();
        Task::ready(Ok(line_edits(&text, range.start.row..range.end.row + 1)))
    }
}

struct OnSemicolon;
impl OnTypeFormattingProvider for OnSemicolon {
    fn auto_format_trigger_characters(&self) -> Vec<char> {
        vec![';']
    }

    fn provide_on_type_formatting_edits(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        ch: char,
        _: FormattingOptions,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<TextEdit>>> {
        assert_eq!(ch, ';');
        let text = context.snapshot.rope().to_string();
        Task::ready(Ok(line_edits(&text, position.row..position.row + 1)))
    }
}

/// Answers when the test says so.
struct Slow(async_channel::Receiver<()>);
impl DocumentFormattingProvider for Slow {
    fn provide_document_formatting_edits(
        &self,
        context: &ProviderContext,
        _: FormattingOptions,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<TextEdit>>> {
        let release = self.0.clone();
        let end = context.snapshot.max_point_utf16();
        cx.background_executor().spawn(async move {
            release.recv().await.ok();
            Ok(vec![(p(0, 0)..end, "formatted".to_string())])
        })
    }
}

fn setup<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_language("rust"));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

fn text(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |e, cx| e.model().read(cx).text())
}

fn set_options(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    f: impl FnOnce(&mut gpui_monaco::EditorOptions),
) {
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        f(&mut options);
        e.update_options(options, cx);
    });
}

#[gpui::test]
fn format_document_applies_minimal_edits_as_one_step(cx: &mut TestAppContext) {
    let (editor, cx) = setup("let  x =  1;  \nfoo(a,  b)", cx);
    cx.update(|_, cx| {
        register_document_formatting_provider(cx, LanguageSelector::All, WholeDocument)
    });
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(1, 9))], cx)
    });
    // Shift+Alt+F.
    let keys = if cfg!(target_os = "linux") {
        "ctrl-shift-i"
    } else {
        "shift-alt-f"
    };
    cx.simulate_keystrokes(keys);
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "let x = 1;\nfoo(a, b)");
    // The cursor followed its text (before `)`), instead of jumping to the
    // end of a whole-document replacement.
    assert_eq!(editor.read_with(cx, |e, _| e.selections()[0].head), p(1, 8));
    editor.update(cx, |e, cx| e.undo(cx));
    assert_eq!(text(&editor, cx), "let  x =  1;  \nfoo(a,  b)");
}

#[gpui::test]
fn format_document_falls_back_to_the_range_formatter(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a  b\nc  d", cx);
    assert!(
        !editor.update(cx, |e, cx| e.format_document(cx)),
        "no formatter"
    );
    cx.update(|_, cx| {
        register_document_range_formatting_provider(cx, LanguageSelector::All, Lines)
    });
    assert!(editor.update(cx, |e, cx| e.format_document(cx)));
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "a b\nc d");
}

#[gpui::test]
fn format_selection_formats_selections_or_the_line(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a  b\nc  d\ne  f\ng  h", cx);
    cx.update(|_, cx| {
        register_document_range_formatting_provider(cx, LanguageSelector::All, Lines)
    });
    // An empty selection formats its line.
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(1, 0))], cx)
    });
    editor.update(cx, |e, cx| e.format_selection(cx));
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "a  b\nc d\ne  f\ng  h");
    // Each selection's lines.
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[
                Selection::new(p(2, 0), p(2, 1)),
                Selection::new(p(3, 1), p(3, 2)),
            ],
            cx,
        )
    });
    editor.update(cx, |e, cx| e.format_selection(cx));
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "a  b\nc d\ne f\ng h");
}

#[gpui::test]
fn formatting_is_dropped_when_the_text_or_cursor_changes(cx: &mut TestAppContext) {
    let (editor, cx) = setup("abc", cx);
    let (release, wait) = async_channel::unbounded();
    cx.update(|_, cx| register_document_formatting_provider(cx, LanguageSelector::All, Slow(wait)));
    // The cursor moves before the result arrives.
    editor.update(cx, |e, cx| e.format_document(cx));
    cx.simulate_keystrokes("right");
    release.try_send(()).unwrap();
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "abc");
    // The text changes.
    editor.update(cx, |e, cx| e.format_document(cx));
    cx.simulate_input("x");
    release.try_send(()).unwrap();
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "axbc");
    // Nothing changes: applied.
    editor.update(cx, |e, cx| e.format_document(cx));
    release.try_send(()).unwrap();
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "formatted");
}

#[gpui::test]
fn format_on_type(cx: &mut TestAppContext) {
    let (editor, cx) = setup("", cx);
    cx.update(|_, cx| register_on_type_formatting_provider(cx, LanguageSelector::All, OnSemicolon));
    // Off by default, as in Monaco.
    cx.simulate_input("let  a;");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "let  a;");
    set_options(&editor, cx, |o| o.format_on_type = true);
    cx.simulate_input("\nlet  b  = 1");
    cx.run_until_parked();
    // Not a trigger character.
    assert_eq!(text(&editor, cx), "let  a;\nlet  b  = 1");
    cx.simulate_input(";");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "let  a;\nlet b = 1;");
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()[0].head),
        p(1, 10)
    );
    // Only with a single cursor.
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[
                Selection::collapsed(p(0, 7)),
                Selection::collapsed(p(1, 10)),
            ],
            cx,
        )
    });
    cx.simulate_input(";");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "let  a;;\nlet b = 1;;");
}

#[gpui::test]
fn format_on_paste(cx: &mut TestAppContext) {
    let (editor, cx) = setup("x  y\n", cx);
    cx.update(|_, cx| {
        register_document_range_formatting_provider(cx, LanguageSelector::All, Lines)
    });
    set_options(&editor, cx, |o| o.format_on_paste = true);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(1, 0))], cx)
    });
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("a  =  1\nb  =  2".into()));
    editor.update_in(cx, |e, window, cx| {
        e.trigger("code_editor::Paste", window, cx).unwrap()
    });
    cx.run_until_parked();
    // Only the pasted lines.
    assert_eq!(text(&editor, cx), "x  y\na = 1\nb = 2");
}

#[gpui::test]
fn read_only_editors_do_not_format(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a  b", cx);
    cx.update(|_, cx| {
        register_document_formatting_provider(cx, LanguageSelector::All, WholeDocument)
    });
    set_options(&editor, cx, |o| o.read_only = true);
    assert!(!editor.update(cx, |e, cx| e.format_document(cx)));
    assert!(!editor.update(cx, |e, cx| e.format_selection(cx)));
}
