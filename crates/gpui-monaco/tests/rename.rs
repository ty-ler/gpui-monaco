//! Rename (Monaco `contrib/rename`) with test providers.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{App, AppContext as _, Entity, Focusable, Task, TestAppContext, VisualTestContext};
use gpui_monaco::cursor::Selection;
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, TextModel};

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

/// Renames every whole-word occurrence, here and in `lib.rs`.
#[derive(Clone, Default)]
struct Words {
    /// Names asked for.
    log: Rc<RefCell<Vec<String>>>,
    /// Resolve with this error instead.
    reject: Option<&'static str>,
    /// Also edit a document nobody can open.
    unknown_document: bool,
}

fn occurrences(text: &str, word: &str) -> Vec<std::ops::Range<PointUtf16>> {
    let mut found = Vec::new();
    for (row, line) in text.split('\n').enumerate() {
        for (start, _) in line.match_indices(word) {
            let end = start + word.len();
            let is_word = |c: char| c.is_alphanumeric() || c == '_';
            if line[..start].chars().next_back().is_some_and(is_word)
                || line[end..].chars().next().is_some_and(is_word)
            {
                continue;
            }
            found.push(p(row as u32, start as u32)..p(row as u32, end as u32));
        }
    }
    found
}

impl RenameProvider for Words {
    fn resolve_rename_location(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: &mut App,
    ) -> Option<Task<anyhow::Result<Option<RenameLocation>>>> {
        let reject = self.reject?;
        Some(Task::ready(Err(anyhow::anyhow!(reject))))
    }

    fn provide_rename_edits(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        new_name: &str,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<WorkspaceEdit>>> {
        self.log.borrow_mut().push(new_name.to_string());
        let text = context.snapshot.rope().to_string();
        let line = context.snapshot.line(position.row);
        let col = position.column as usize;
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        let start = line[..col]
            .rfind(|c: char| !is_word(c))
            .map_or(0, |i| i + 1);
        let end = line[col..]
            .find(|c: char| !is_word(c))
            .map_or(line.len(), |i| col + i);
        let word = &line[start..end];
        let edits = |text: &str| -> Vec<TextEdit> {
            occurrences(text, word)
                .into_iter()
                .map(|range| (range, new_name.to_string()))
                .collect()
        };
        let mut edit = WorkspaceEdit {
            edits: vec![
                (None, edits(&text)),
                (Some("file:///lib.rs".into()), edits("pub fn foo() {}")),
            ],
            ..Default::default()
        };
        if self.unknown_document {
            edit.edits
                .push((Some("file:///missing.rs".into()), Vec::new()));
        }
        Task::ready(Ok(Some(edit)))
    }
}

/// Passes to the next provider.
struct Nothing;
impl RenameProvider for Nothing {
    fn provide_rename_edits(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: &str,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<WorkspaceEdit>>> {
        Task::ready(Ok(None))
    }
}

/// Registers providers.
type Register = Box<dyn Fn(&mut App)>;

const TEXT: &str = "fn foo() {}\nfn main() {\n    foo();\n}";

fn setup(
    providers: Vec<Register>,
    cx: &mut TestAppContext,
) -> (
    Entity<CodeEditor>,
    Entity<TextModel>,
    &mut VisualTestContext,
) {
    cx.update(gpui_monaco::init);
    let lib = cx.new(|_| TextModel::new("pub fn foo() {}").with_uri("file:///lib.rs"));
    cx.update(|cx| {
        gpui_monaco::register_model(cx, &lib);
        for register in &providers {
            register(cx);
        }
    });
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(TEXT).with_uri("file:///main.rs"));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    draw(cx);
    (editor, lib, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
}

fn text(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |e, cx| e.model().read(cx).text())
}

fn words(words: Words) -> Register {
    Box::new(move |cx| {
        register_rename_provider(cx, LanguageSelector::All, words.clone());
    })
}

#[gpui::test]
fn f2_renames_across_documents(cx: &mut TestAppContext) {
    let provider = Words::default();
    let log = provider.log.clone();
    let (editor, lib, cx) = setup(vec![words(provider)], cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(2, 5))], cx)
    });
    cx.simulate_keystrokes("f2");
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_rename_visible()));
    // The name is selected in the box: typing replaces it.
    cx.simulate_input("bar");
    cx.simulate_keystrokes("enter");
    draw(cx);
    assert_eq!(*log.borrow(), ["bar"]);
    assert!(!editor.read_with(cx, |e, _| e.is_rename_visible()));
    assert_eq!(text(&editor, cx), "fn bar() {}\nfn main() {\n    bar();\n}");
    assert_eq!(lib.read_with(cx, |m, _| m.text()), "pub fn bar() {}");
    // Monaco alerts screen readers with the bulk edit's summary.
    assert_eq!(
        editor.read_with(cx, |e, _| e.last_announcement_for_tests()),
        Some("Successfully renamed 'foo' to 'bar'. Summary: Made 3 edits in 2 files".into())
    );
    // The editor has focus back; one undo reverts it here.
    assert!(cx.update(|window, cx| editor.focus_handle(cx).is_focused(window)));
    editor.update(cx, |e, cx| e.undo(cx));
    assert_eq!(text(&editor, cx), TEXT);
}

#[gpui::test]
fn escape_and_unchanged_names_do_nothing(cx: &mut TestAppContext) {
    let provider = Words::default();
    let log = provider.log.clone();
    let (editor, _, cx) = setup(vec![words(provider)], cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 4))], cx)
    });
    cx.simulate_keystrokes("f2");
    draw(cx);
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_rename_visible()));
    assert!(cx.update(|window, cx| editor.focus_handle(cx).is_focused(window)));
    cx.simulate_keystrokes("f2");
    draw(cx);
    cx.simulate_keystrokes("enter");
    draw(cx);
    assert!(log.borrow().is_empty());
    assert_eq!(text(&editor, cx), TEXT);
}

#[gpui::test]
fn rejections_and_failures_show_messages(cx: &mut TestAppContext) {
    // A provider that can't rename here says why.
    let (editor, _, cx) = setup(
        vec![words(Words {
            reject: Some("You cannot rename this element"),
            ..Default::default()
        })],
        cx,
    );
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 4))], cx)
    });
    cx.simulate_keystrokes("f2");
    draw(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_rename_visible()));
    assert!(editor.read_with(cx, |e, _| e.is_message_visible()));
}

#[gpui::test]
fn providers_pass_on_and_unknown_documents_fail(cx: &mut TestAppContext) {
    let provider = Words {
        unknown_document: true,
        ..Default::default()
    };
    let log = provider.log.clone();
    let nothing: Register = Box::new(|cx| {
        register_rename_provider(cx, LanguageSelector::All, Nothing);
    });
    let (editor, lib, cx) = setup(vec![nothing, words(provider)], cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 4))], cx)
    });
    cx.simulate_keystrokes("f2");
    draw(cx);
    cx.simulate_input("baz");
    cx.simulate_keystrokes("enter");
    draw(cx);
    // The second provider answered, but its edit names a document that
    // can't be found: nothing changes and a message says so.
    assert_eq!(*log.borrow(), ["baz"]);
    assert_eq!(text(&editor, cx), TEXT);
    assert_eq!(lib.read_with(cx, |m, _| m.text()), "pub fn foo() {}");
    assert!(editor.read_with(cx, |e, _| e.is_message_visible()));
}

/// Suggests fixed names (Monaco `NewSymbolNamesProvider`).
#[derive(Clone, Default)]
struct Suggest {
    names: Vec<&'static str>,
    automatic: bool,
    /// The trigger kinds asked with.
    log: Rc<RefCell<Vec<NewSymbolNameTriggerKind>>>,
}

impl NewSymbolNamesProvider for Suggest {
    fn supports_automatic_new_symbol_names_trigger_kind(&self) -> bool {
        self.automatic
    }

    fn provide_new_symbol_names(
        &self,
        _: &ProviderContext,
        range: std::ops::Range<PointUtf16>,
        trigger: NewSymbolNameTriggerKind,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<NewSymbolName>>> {
        // Asked about the symbol's range.
        assert_eq!(range, p(2, 4)..p(2, 7));
        self.log.borrow_mut().push(trigger);
        Task::ready(Ok(self
            .names
            .iter()
            .map(|n| NewSymbolName::new(*n))
            .collect()))
    }
}

fn suggest(suggest: Suggest) -> Register {
    Box::new(move |cx| {
        register_new_symbol_name_provider(cx, LanguageSelector::All, suggest.clone());
    })
}

fn input(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> (String, Option<usize>) {
    editor
        .read_with(cx, |e, cx| e.rename_input_for_tests(cx))
        .unwrap()
}

#[gpui::test]
fn suggestions_are_listed_and_picked_with_up_and_down(cx: &mut TestAppContext) {
    let provider = Words::default();
    let log = provider.log.clone();
    let names = Suggest {
        // The current name, blanks and repeats are dropped.
        names: vec!["fooBar", "foo", " ", "baz", "fooBar"],
        automatic: true,
        ..Default::default()
    };
    let asked = names.log.clone();
    let (editor, _lib, cx) = setup(vec![words(provider), suggest(names)], cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(2, 5))], cx)
    });
    cx.simulate_keystrokes("f2");
    draw(cx);
    // Asked as soon as the box opened.
    assert_eq!(*asked.borrow(), [NewSymbolNameTriggerKind::Automatic]);
    assert_eq!(
        editor.read_with(cx, |e, cx| e.rename_suggestions(cx)),
        ["fooBar", "baz"]
    );
    // The button shows; the preview label doesn't without a handler.
    assert_eq!(
        editor.read_with(cx, |e, cx| e.rename_box_extras_for_tests(cx)),
        Some((false, true))
    );
    // Monaco `focusNextRenameSuggestion`: the box shows the focused one,
    // and the current name again past the last.
    cx.simulate_keystrokes("down");
    draw(cx);
    assert_eq!(input(&editor, cx), ("fooBar".into(), Some(0)));
    cx.simulate_keystrokes("down");
    draw(cx);
    assert_eq!(input(&editor, cx), ("baz".into(), Some(1)));
    cx.simulate_keystrokes("down");
    draw(cx);
    assert_eq!(input(&editor, cx), ("foo".into(), None));
    cx.simulate_keystrokes("up");
    draw(cx);
    assert_eq!(input(&editor, cx), ("baz".into(), Some(1)));
    cx.simulate_keystrokes("enter");
    draw(cx);
    assert_eq!(*log.borrow(), ["baz"]);
    assert_eq!(text(&editor, cx), "fn baz() {}\nfn main() {\n    baz();\n}");
}

#[gpui::test]
fn typing_unfocuses_suggestions_and_the_button_asks_again(cx: &mut TestAppContext) {
    let provider = Words::default();
    let log = provider.log.clone();
    let names = Suggest {
        names: vec!["qux"],
        automatic: false,
        ..Default::default()
    };
    let asked = names.log.clone();
    let (editor, _lib, cx) = setup(vec![words(provider), suggest(names)], cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(2, 5))], cx)
    });
    cx.simulate_keystrokes("f2");
    draw(cx);
    // Not automatic: nothing until the button is clicked.
    assert!(asked.borrow().is_empty());
    assert!(
        editor
            .read_with(cx, |e, cx| e.rename_suggestions(cx))
            .is_empty()
    );
    // With no suggestions, Down puts the current name back.
    cx.simulate_input("x");
    cx.simulate_keystrokes("down");
    draw(cx);
    assert_eq!(input(&editor, cx), ("foo".into(), None));
    editor.update(cx, |e, cx| e.request_rename_suggestions(cx));
    draw(cx);
    assert_eq!(*asked.borrow(), [NewSymbolNameTriggerKind::Invoke]);
    assert_eq!(
        editor.read_with(cx, |e, cx| e.rename_suggestions(cx)),
        ["qux"]
    );
    cx.simulate_keystrokes("down");
    draw(cx);
    assert_eq!(input(&editor, cx), ("qux".into(), Some(0)));
    // Typing edits the name instead of picking the suggestion.
    cx.simulate_input("2");
    draw(cx);
    assert_eq!(input(&editor, cx), ("qux2".into(), None));
    cx.simulate_keystrokes("enter");
    draw(cx);
    assert_eq!(*log.borrow(), ["qux2"]);
}

#[gpui::test]
fn previews_go_through_the_preview_handler(cx: &mut TestAppContext) {
    let provider = Words::default();
    let (editor, lib, cx) = setup(vec![words(provider)], cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 4))], cx)
    });
    // Without a handler, Cmd/Ctrl+Enter just renames.
    cx.simulate_keystrokes("f2");
    draw(cx);
    cx.simulate_input("bar");
    cx.simulate_keystrokes("secondary-enter");
    draw(cx);
    assert_eq!(text(&editor, cx), "fn bar() {}\nfn main() {\n    bar();\n}");
    assert_eq!(lib.read_with(cx, |m, _| m.text()), "pub fn bar() {}");

    // The handler sees the edit and its label, and here keeps only this
    // document's edits.
    let previews = Rc::new(RefCell::new(Vec::new()));
    let seen = previews.clone();
    let handler = cx.update(|_, cx| {
        gpui_monaco::set_workspace_edit_preview_handler(cx, move |mut edit, label, _| {
            seen.borrow_mut().push(label.to_string());
            edit.edits.retain(|(uri, _)| uri.is_none());
            Task::ready(Ok(Some(edit)))
        })
    });
    cx.simulate_keystrokes("f2");
    draw(cx);
    // The label says how to preview.
    assert_eq!(
        editor.read_with(cx, |e, cx| e.rename_box_extras_for_tests(cx)),
        Some((true, false))
    );
    cx.simulate_input("baz");
    cx.simulate_keystrokes("secondary-enter");
    draw(cx);
    assert_eq!(*previews.borrow(), ["Renaming 'bar' to 'baz'"]);
    assert_eq!(text(&editor, cx), "fn baz() {}\nfn main() {\n    baz();\n}");
    assert_eq!(lib.read_with(cx, |m, _| m.text()), "pub fn bar() {}");
    // Enter doesn't preview.
    cx.simulate_keystrokes("f2");
    draw(cx);
    cx.simulate_input("qux");
    cx.simulate_keystrokes("enter");
    draw(cx);
    assert_eq!(previews.borrow().len(), 1);
    assert_eq!(text(&editor, cx), "fn qux() {}\nfn main() {\n    qux();\n}");
    cx.update(|_, cx| handler.dispose(cx));
}

/// Resolves rename locations as told, counting the requests.
struct Resolves {
    count: Rc<std::cell::Cell<usize>>,
    location: Option<RenameLocation>,
}

impl RenameProvider for Resolves {
    fn resolve_rename_location(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: &mut App,
    ) -> Option<Task<anyhow::Result<Option<RenameLocation>>>> {
        self.count.set(self.count.get() + 1);
        Some(Task::ready(Ok(self.location.clone())))
    }

    fn provide_rename_edits(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: &str,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<WorkspaceEdit>>> {
        Task::ready(Ok(None))
    }
}

/// Monaco `RenameSkeleton.resolveRenameLocation`: one provider at a time,
/// best (newest) first, stopping at the first location.
#[gpui::test]
fn rename_locations_are_resolved_one_provider_at_a_time(cx: &mut TestAppContext) {
    let older = Rc::new(std::cell::Cell::new(0));
    let newer = Rc::new(std::cell::Cell::new(0));
    let (o, n) = (older.clone(), newer.clone());
    let register: Register = Box::new(move |cx| {
        register_rename_provider(
            cx,
            LanguageSelector::All,
            Resolves {
                count: o.clone(),
                location: None,
            },
        );
        register_rename_provider(
            cx,
            LanguageSelector::All,
            Resolves {
                count: n.clone(),
                location: Some(RenameLocation {
                    range: p(0, 3)..p(0, 6),
                    text: "foo".into(),
                }),
            },
        );
    });
    let (editor, _, cx) = setup(vec![register], cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 4))], cx)
    });
    cx.simulate_keystrokes("f2");
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_rename_visible()));
    assert_eq!((newer.get(), older.get()), (1, 0));
}

fn apply(
    editor: &Entity<CodeEditor>,
    edit: WorkspaceEdit,
    cx: &mut VisualTestContext,
) -> anyhow::Result<()> {
    let task = editor.update(cx, |e, cx| e.apply_workspace_edit(edit, cx));
    let result = Rc::new(RefCell::new(None));
    let out = result.clone();
    cx.update(|_, cx| {
        cx.spawn(async move |_| *out.borrow_mut() = Some(task.await))
            .detach()
    });
    draw(cx);
    result.borrow_mut().take().expect("applied")
}

/// Monaco `IWorkspaceFileEdit`s: standalone Monaco rejects them; here
/// they go to the host's handler first, then the text edits apply.
#[gpui::test]
fn file_operations_go_to_the_host(cx: &mut TestAppContext) {
    let (editor, _, cx) = setup(Vec::new(), cx);
    let edit = WorkspaceEdit {
        edits: vec![(None, vec![(p(0, 3)..p(0, 6), "bar".into())])],
        file_edits: vec![WorkspaceFileEdit {
            old_uri: Some("file:///old.rs".into()),
            new_uri: Some("file:///new.rs".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    assert!(apply(&editor, edit.clone(), cx).is_err());
    assert_eq!(text(&editor, cx), TEXT);

    let renamed = Rc::new(RefCell::new(Vec::new()));
    let seen = renamed.clone();
    let handler = cx.update(|_, cx| {
        gpui_monaco::set_workspace_file_edit_handler(cx, move |edits, _| {
            seen.borrow_mut().extend(edits);
            Task::ready(Ok(()))
        })
    });
    apply(&editor, edit.clone(), cx).unwrap();
    assert_eq!(renamed.borrow().len(), 1);
    assert_eq!(
        renamed.borrow()[0].new_uri.as_deref(),
        Some("file:///new.rs")
    );
    assert_eq!(text(&editor, cx), "fn bar() {}\nfn main() {\n    foo();\n}");
    cx.update(|_, cx| handler.dispose(cx));
}

/// Monaco `StandaloneBulkEditService`: "bad state - model changed in the
/// meantime" when an edit's version is stale.
#[gpui::test]
fn stale_versions_are_rejected(cx: &mut TestAppContext) {
    let (editor, _, cx) = setup(Vec::new(), cx);
    let version = editor.read_with(cx, |e, cx| e.model().read(cx).version());
    let edit = |version| WorkspaceEdit {
        edits: vec![(None, vec![(p(0, 3)..p(0, 6), "bar".into())])],
        versions: vec![(None, version)],
        ..Default::default()
    };
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(3, 1))], cx)
    });
    cx.simulate_input("x");
    let error = apply(&editor, edit(version), cx).unwrap_err();
    assert_eq!(
        error.to_string(),
        "bad state - model changed in the meantime"
    );
    let version = editor.read_with(cx, |e, cx| e.model().read(cx).version());
    apply(&editor, edit(version), cx).unwrap();
    assert!(text(&editor, cx).starts_with("fn bar() {}"));
}

/// Monaco `insertAsSnippet` text edits: a single one in the editor starts
/// a snippet session; other text edits apply with it as one undo step.
#[gpui::test]
fn snippet_edits_start_a_snippet_session(cx: &mut TestAppContext) {
    let (editor, lib, cx) = setup(Vec::new(), cx);
    let edit = WorkspaceEdit {
        edits: vec![(None, vec![(p(2, 4)..p(2, 7), "renamed".into())])],
        snippet_edits: vec![
            WorkspaceSnippetEdit {
                range: p(0, 3)..p(0, 6),
                snippet: "${1:renamed}_fn".into(),
                ..Default::default()
            },
            // Elsewhere, the snippet's text is inserted.
            WorkspaceSnippetEdit {
                uri: Some("file:///lib.rs".into()),
                range: p(0, 7)..p(0, 10),
                snippet: "${1:x}y$0".into(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    apply(&editor, edit, cx).unwrap();
    assert_eq!(
        text(&editor, cx),
        "fn renamed_fn() {}\nfn main() {\n    renamed();\n}"
    );
    assert_eq!(lib.read_with(cx, |m, _| m.text()), "pub fn xy() {}");
    // The placeholder is selected.
    assert!(editor.read_with(cx, |e, _| e.in_snippet_mode()));
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()[0].range()),
        p(0, 3)..p(0, 10)
    );
    editor.update(cx, |e, cx| e.undo(cx));
    assert_eq!(text(&editor, cx), TEXT);
}

/// Monaco's bulk edit service previews edits whose metadata needs
/// confirmation, even without Cmd/Ctrl+Enter.
#[gpui::test]
fn edits_needing_confirmation_are_previewed(cx: &mut TestAppContext) {
    struct Confirm;
    impl RenameProvider for Confirm {
        fn provide_rename_edits(
            &self,
            _: &ProviderContext,
            _: PointUtf16,
            new_name: &str,
            _: &mut App,
        ) -> Task<anyhow::Result<Option<WorkspaceEdit>>> {
            Task::ready(Ok(Some(WorkspaceEdit {
                edits: vec![(None, vec![(p(0, 3)..p(0, 6), new_name.to_string())])],
                metadata: Some(WorkspaceEditMetadata {
                    needs_confirmation: true,
                    label: "Rename".into(),
                    description: None,
                }),
                ..Default::default()
            })))
        }
    }
    let register: Register = Box::new(|cx| {
        register_rename_provider(cx, LanguageSelector::All, Confirm);
    });
    let (editor, _, cx) = setup(vec![register], cx);
    let previews = Rc::new(std::cell::Cell::new(0));
    let seen = previews.clone();
    let handler = cx.update(|_, cx| {
        gpui_monaco::set_workspace_edit_preview_handler(cx, move |_, _, _| {
            seen.set(seen.get() + 1);
            // The user cancels.
            Task::ready(Ok(None))
        })
    });
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 4))], cx)
    });
    cx.simulate_keystrokes("f2");
    draw(cx);
    cx.simulate_input("bar");
    cx.simulate_keystrokes("enter");
    draw(cx);
    assert_eq!(previews.get(), 1);
    assert_eq!(text(&editor, cx), TEXT);
    cx.update(|_, cx| handler.dispose(cx));
}
