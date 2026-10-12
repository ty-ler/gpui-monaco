//! Formatting (Monaco `contrib/format`) with test formatters.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use gpui::{App, AppContext as _, Entity, Focusable, Task, TestAppContext, VisualTestContext};
use gpui_monaco::cursor::Selection;
use gpui_monaco::providers::*;
use gpui_monaco::text::{LineEnding, PointUtf16};
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
    ) -> Task<anyhow::Result<FormattingEdits>> {
        // A whole-text replacement, like many formatters return.
        let text = context.snapshot.rope().to_string();
        let tidy: Vec<String> = text.split('\n').map(tidy).collect();
        let end = context.snapshot.max_point_utf16();
        Task::ready(Ok(vec![(p(0, 0)..end, tidy.join("\n"))].into()))
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
    ) -> Task<anyhow::Result<FormattingEdits>> {
        let text = context.snapshot.rope().to_string();
        Task::ready(Ok(
            line_edits(&text, range.start.row..range.end.row + 1).into()
        ))
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
    ) -> Task<anyhow::Result<FormattingEdits>> {
        assert_eq!(ch, ';');
        let text = context.snapshot.rope().to_string();
        Task::ready(Ok(line_edits(&text, position.row..position.row + 1).into()))
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
    ) -> Task<anyhow::Result<FormattingEdits>> {
        let release = self.0.clone();
        let end = context.snapshot.max_point_utf16();
        cx.background_executor().spawn(async move {
            release.recv().await.ok();
            Ok(vec![(p(0, 0)..end, "formatted".to_string())].into())
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

/// Replaces the whole text with its name, as `name` from `extension`.
struct Named(&'static str, Option<&'static str>);
impl DocumentFormattingProvider for Named {
    fn provide_document_formatting_edits(
        &self,
        context: &ProviderContext,
        _: FormattingOptions,
        _: &mut App,
    ) -> Task<anyhow::Result<FormattingEdits>> {
        let end = context.snapshot.max_point_utf16();
        Task::ready(Ok(vec![(p(0, 0)..end, self.0.to_string())].into()))
    }

    fn display_name(&self) -> Option<gpui::SharedString> {
        Some(self.0.into())
    }

    fn extension_id(&self) -> Option<gpui::SharedString> {
        self.1.map(Into::into)
    }
}

/// Replaces the range it's given with its name.
struct NamedRange(&'static str, Option<&'static str>);
impl DocumentRangeFormattingProvider for NamedRange {
    fn provide_document_range_formatting_edits(
        &self,
        _: &ProviderContext,
        range: Range<PointUtf16>,
        _: FormattingOptions,
        _: &mut App,
    ) -> Task<anyhow::Result<FormattingEdits>> {
        Task::ready(Ok(vec![(range, self.0.to_string())].into()))
    }

    fn display_name(&self) -> Option<gpui::SharedString> {
        Some(self.0.into())
    }

    fn extension_id(&self) -> Option<gpui::SharedString> {
        self.1.map(Into::into)
    }
}

type Asked = Rc<RefCell<Vec<(Vec<Option<String>>, FormattingMode, FormattingKind)>>>;

/// A selector recording what it's asked and answering `choice`.
fn record_selector(
    choice: Option<usize>,
    cx: &mut VisualTestContext,
) -> (Asked, gpui_monaco::Disposable) {
    let asked: Asked = Default::default();
    let seen = asked.clone();
    let disposable = cx.update(|_, cx| {
        set_formatter_selector(cx, move |selection, _| {
            let names = selection
                .formatters
                .iter()
                .map(|f| f.display_name.as_ref().map(|n| n.to_string()))
                .collect();
            seen.borrow_mut()
                .push((names, selection.mode, selection.kind));
            Task::ready(choice)
        })
    });
    (asked, disposable)
}

/// Monaco `FormattingConflicts`: without a selector the first formatter
/// runs (standalone `editor.api.ts`); a selector chooses among the
/// document formatters and the range formatters of other extensions.
#[gpui::test]
fn choosing_between_document_formatters(cx: &mut TestAppContext) {
    let (editor, cx) = setup("text", cx);
    // Registered in reverse: equal scores are tried newest first, as in
    // Monaco's `LanguageFeatureRegistry.ordered`.
    cx.update(|_, cx| {
        register_document_formatting_provider(cx, LanguageSelector::All, Named("two", None));
        register_document_formatting_provider(cx, LanguageSelector::All, Named("one", Some("a")));
        register_document_range_formatting_provider(
            cx,
            LanguageSelector::All,
            NamedRange("three", Some("b")),
        );
        // Extension `a` already formats documents: not offered again.
        register_document_range_formatting_provider(
            cx,
            LanguageSelector::All,
            NamedRange("one range", Some("a")),
        );
    });
    assert!(editor.update(cx, |e, cx| e.format_document(cx)));
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "one");

    let (asked, disposable) = record_selector(Some(2), cx);
    editor.update(cx, |e, cx| e.format_document(cx));
    cx.run_until_parked();
    // The range formatter formats the whole text.
    assert_eq!(text(&editor, cx), "three");
    assert_eq!(
        *asked.borrow(),
        [(
            vec![
                Some("one".to_string()),
                Some("two".to_string()),
                Some("three".to_string())
            ],
            FormattingMode::Explicit,
            FormattingKind::File
        )]
    );
    cx.update(|_, cx| disposable.dispose(cx));

    // Choosing none formats nothing.
    let (asked, disposable) = record_selector(None, cx);
    editor.update(cx, |e, cx| e.format_document(cx));
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "three");
    assert_eq!(asked.borrow().len(), 1);
    // The newest selector wins; disposing it brings back the previous one.
    let (newer, newer_disposable) = record_selector(Some(1), cx);
    editor.update(cx, |e, cx| e.format_document(cx));
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "two");
    assert_eq!((asked.borrow().len(), newer.borrow().len()), (1, 1));
    cx.update(|_, cx| newer_disposable.dispose(cx));
    editor.update(cx, |e, cx| e.format_document(cx));
    cx.run_until_parked();
    assert_eq!(asked.borrow().len(), 2);
    cx.update(|_, cx| disposable.dispose(cx));
}

/// Monaco `LanguageFeatureRegistry.ordered`: a formatter for the model's
/// language beats a `*` one whenever they were registered, and among
/// equals the newest runs.
#[gpui::test]
fn the_best_ranked_formatter_runs(cx: &mut TestAppContext) {
    let (editor, cx) = setup("text", cx);
    cx.update(|_, cx| {
        register_document_formatting_provider(
            cx,
            LanguageSelector::language("rust"),
            Named("rust", None),
        );
        register_document_formatting_provider(cx, LanguageSelector::All, Named("any", None));
    });
    editor.update(cx, |e, cx| e.format_document(cx));
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "rust");
    cx.update(|_, cx| {
        register_document_formatting_provider(
            cx,
            LanguageSelector::language("rust"),
            Named("newer", None),
        );
    });
    editor.update(cx, |e, cx| e.format_document(cx));
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "newer");
}

/// A selector answering later, e.g. after asking the user: the choice runs
/// then, unless the text changed meanwhile.
#[gpui::test]
fn choosing_a_formatter_asynchronously(cx: &mut TestAppContext) {
    let (editor, cx) = setup("text", cx);
    // Newest first.
    cx.update(|_, cx| {
        register_document_formatting_provider(cx, LanguageSelector::All, Named("two", None));
        register_document_formatting_provider(cx, LanguageSelector::All, Named("one", None));
    });
    let (send, receive) = async_channel::unbounded::<Option<usize>>();
    let disposable = cx.update(|_, cx| {
        set_formatter_selector(cx, move |_, cx| {
            let receive = receive.clone();
            cx.background_executor()
                .spawn(async move { receive.recv().await.ok().flatten() })
        })
    });
    editor.update(cx, |e, cx| e.format_document(cx));
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "text");
    send.try_send(Some(1)).unwrap();
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "two");
    cx.update(|_, cx| disposable.dispose(cx));
}

/// Selections and pasted text choose among the range formatters, silently
/// for pastes (Monaco `FormattingMode.Silent`).
#[gpui::test]
fn choosing_between_range_formatters(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a\nb\n", cx);
    // Newest first.
    cx.update(|_, cx| {
        register_document_range_formatting_provider(
            cx,
            LanguageSelector::All,
            NamedRange("y", None),
        );
        register_document_range_formatting_provider(
            cx,
            LanguageSelector::All,
            NamedRange("x", None),
        );
    });
    let (asked, disposable) = record_selector(Some(1), cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::new(p(0, 0), p(0, 1))], cx);
        e.format_selection(cx);
    });
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "y\nb\n");
    set_options(&editor, cx, |o| o.format_on_paste = true);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(2, 0))], cx)
    });
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("pasted".into()));
    editor.update_in(cx, |e, window, cx| {
        e.trigger("code_editor::Paste", window, cx).unwrap()
    });
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "y\nb\ny");
    let names = vec![Some("x".to_string()), Some("y".to_string())];
    assert_eq!(
        *asked.borrow(),
        [
            (
                names.clone(),
                FormattingMode::Explicit,
                FormattingKind::Selection
            ),
            (names, FormattingMode::Silent, FormattingKind::Selection),
        ]
    );
    cx.update(|_, cx| disposable.dispose(cx));
}

/// Returns fixed edits and line ending.
struct Fixed(Vec<TextEdit>, Option<LineEnding>);
impl DocumentFormattingProvider for Fixed {
    fn provide_document_formatting_edits(
        &self,
        _: &ProviderContext,
        _: FormattingOptions,
        _: &mut App,
    ) -> Task<anyhow::Result<FormattingEdits>> {
        Task::ready(Ok(FormattingEdits {
            edits: self.0.clone(),
            eol: self.1,
        }))
    }
}

/// Monaco `FormattingEdit.execute`: `EditOperation.replaceMove`, so a
/// cursor at an edit's start ends after its text, except for a single
/// whole-text replacement (audit 3 §A.19).
#[gpui::test]
fn format_edits_move_cursors_past_their_text(cx: &mut TestAppContext) {
    let (editor, cx) = setup("\tfoo", cx);
    cx.update(|_, cx| {
        register_document_formatting_provider(
            cx,
            LanguageSelector::All,
            Fixed(vec![(p(0, 0)..p(0, 1), "    ".into())], None),
        )
    });
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 0))], cx)
    });
    editor.update(cx, |e, cx| e.format_document(cx));
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "    foo");
    assert_eq!(editor.read_with(cx, |e, _| e.selections()[0].head), p(0, 4));
}

/// Monaco `FormattingEdit.execute`: "replace semantics" for one edit
/// replacing the whole text.
#[gpui::test]
fn whole_text_format_edits_keep_cursors(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a", cx);
    cx.update(|_, cx| {
        register_document_formatting_provider(cx, LanguageSelector::All, Named("b", None))
    });
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 0))], cx)
    });
    editor.update(cx, |e, cx| e.format_document(cx));
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "b");
    assert_eq!(editor.read_with(cx, |e, _| e.selections()[0].head), p(0, 0));
}

/// Monaco `FormattingEdit._handleEolEdits`: a formatter's line ending is
/// pushed with its edits, in one undo step (audit 3 §A.21).
#[gpui::test]
fn formatters_change_the_line_ending(cx: &mut TestAppContext) {
    let (editor, cx) = setup("x\ny", cx);
    cx.update(|_, cx| {
        register_document_formatting_provider(
            cx,
            LanguageSelector::All,
            Fixed(vec![(p(0, 0)..p(0, 1), "z".into())], Some(LineEnding::Crlf)),
        )
    });
    let eol = |editor: &Entity<CodeEditor>, cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, cx| e.model().read(cx).line_ending())
    };
    editor.update(cx, |e, cx| e.format_document(cx));
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "z\r\ny");
    assert_eq!(eol(&editor, cx), LineEnding::Crlf);
    editor.update(cx, |e, cx| e.undo(cx));
    assert_eq!(text(&editor, cx), "x\ny");
    assert_eq!(eol(&editor, cx), LineEnding::Lf);
}

/// A formatter's line ending alone is applied too.
#[gpui::test]
fn formatters_change_only_the_line_ending(cx: &mut TestAppContext) {
    let (editor, cx) = setup("x\ny", cx);
    cx.update(|_, cx| {
        register_document_formatting_provider(
            cx,
            LanguageSelector::All,
            Fixed(Vec::new(), Some(LineEnding::Crlf)),
        )
    });
    editor.update(cx, |e, cx| e.format_document(cx));
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "x\r\ny");
}

/// Monaco `FormatOnPaste._trigger`: nothing is formatted with several
/// cursors (audit 3 §A.20).
#[gpui::test]
fn format_on_paste_needs_a_single_cursor(cx: &mut TestAppContext) {
    let (editor, cx) = setup("x  y\n\n", cx);
    cx.update(|_, cx| {
        register_document_range_formatting_provider(cx, LanguageSelector::All, Lines)
    });
    set_options(&editor, cx, |o| o.format_on_paste = true);
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[Selection::collapsed(p(1, 0)), Selection::collapsed(p(2, 0))],
            cx,
        )
    });
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("a  =  1".into()));
    editor.update_in(cx, |e, window, cx| {
        e.trigger("code_editor::Paste", window, cx).unwrap()
    });
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "x  y\na  =  1\na  =  1");
}

/// Records the options it was given.
struct RecordOptions(Rc<RefCell<Vec<FormattingOptions>>>);
impl DocumentFormattingProvider for RecordOptions {
    fn provide_document_formatting_edits(
        &self,
        _: &ProviderContext,
        options: FormattingOptions,
        _: &mut App,
    ) -> Task<anyhow::Result<FormattingEdits>> {
        self.0.borrow_mut().push(options);
        Task::ready(Ok(Vec::new().into()))
    }
}

#[gpui::test]
fn formatters_get_the_indent_size(cx: &mut TestAppContext) {
    // Monaco `TextModel.getFormattingOptions`: `{ tabSize: indentSize,
    // insertSpaces }`.
    let (editor, cx) = setup("a", cx);
    let seen = Rc::new(RefCell::new(Vec::new()));
    cx.update(|_, cx| {
        register_document_formatting_provider(
            cx,
            LanguageSelector::All,
            RecordOptions(seen.clone()),
        )
    });
    let model = editor.read_with(cx, |e, _| e.model().clone());
    model.update(cx, |m, cx| {
        let options = gpui_monaco::ModelOptions {
            tab_size: 8,
            indent_size: 2,
            indent_size_is_tab_size: false,
            insert_spaces: true,
            ..m.options().clone()
        };
        m.update_options(options, cx);
    });
    assert!(editor.update(cx, |e, cx| e.format_document(cx)));
    cx.run_until_parked();
    // `indentSize: 'tabSize'` resolves to the tab size.
    model.update(cx, |m, cx| {
        let options = gpui_monaco::ModelOptions {
            indent_size_is_tab_size: true,
            insert_spaces: false,
            ..m.options().clone()
        };
        m.update_options(options, cx);
    });
    assert!(editor.update(cx, |e, cx| e.format_document(cx)));
    cx.run_until_parked();
    assert_eq!(
        *seen.borrow(),
        [
            FormattingOptions {
                tab_size: 2,
                insert_spaces: true
            },
            FormattingOptions {
                tab_size: 8,
                insert_spaces: false
            },
        ]
    );
}
