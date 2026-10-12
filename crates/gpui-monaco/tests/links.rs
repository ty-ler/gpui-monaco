//! Links (Monaco `contrib/links`): detection, providers, following.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, AppContext as _, Entity, Focusable, SharedString, Task, TestAppContext, VisualTestContext,
};
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, CodeEditorEvent, EditorOptions, TextModel};

fn setup<'a>(
    text: &str,
    uri: &str,
    options: impl FnOnce(&mut EditorOptions),
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let uri = uri.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_uri(uri));
        CodeEditor::new(model, window, cx)
    });
    editor.update(cx, |editor, cx| {
        let mut opts = editor.options().clone();
        options(&mut opts);
        editor.update_options(opts, cx);
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

type Links = Vec<(std::ops::Range<PointUtf16>, Option<SharedString>)>;

fn links(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Links {
    cx.run_until_parked();
    editor.read_with(cx, |e, cx| e.links_for_tests(cx))
}

fn link(
    row: u32,
    start: u32,
    end: u32,
    url: &str,
) -> (std::ops::Range<PointUtf16>, Option<SharedString>) {
    (
        PointUtf16::new(row, start)..PointUtf16::new(row, end),
        Some(url.to_string().into()),
    )
}

/// Collects the links followed, with a link opener that takes them all.
fn opened(_: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Rc<RefCell<Vec<String>>> {
    let opened = Rc::new(RefCell::new(Vec::new()));
    let sink = opened.clone();
    cx.update(|_, cx| {
        let _opener = gpui_monaco::register_link_opener(cx, move |url, _| {
            sink.borrow_mut().push(url.to_string());
            true
        });
    });
    opened
}

/// Collects `OpenLink` events: links nothing opened.
fn reported(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Rc<RefCell<Vec<String>>> {
    let reported = Rc::new(RefCell::new(Vec::new()));
    let sink = reported.clone();
    cx.update(|_, cx| {
        cx.subscribe(editor, move |_, event, _| {
            if let CodeEditorEvent::OpenLink(url) = event {
                sink.borrow_mut().push(url.to_string());
            }
        })
        .detach()
    });
    reported
}

#[gpui::test]
fn detects_urls_and_recomputes_after_edits(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        "// see https://example.com/a.\nlet x = \"http://b.org\";\n",
        "file:///main.rs",
        |_| {},
        cx,
    );
    assert_eq!(
        links(&editor, cx),
        vec![
            link(0, 7, 28, "https://example.com/a"),
            link(1, 9, 21, "http://b.org"),
        ]
    );
    // Recomputed a second after an edit.
    editor.update(cx, |e, cx| {
        e.model()
            .update(cx, |m, cx| m.edit([(0..0, "https://c.dev ")], cx).unwrap());
    });
    cx.executor().advance_clock(Duration::from_millis(500));
    assert_eq!(links(&editor, cx).len(), 2);
    cx.executor().advance_clock(Duration::from_millis(600));
    assert_eq!(links(&editor, cx)[0], link(0, 0, 13, "https://c.dev"));
}

#[gpui::test]
fn links_option(cx: &mut TestAppContext) {
    let (editor, cx) = setup("https://a.io", "file:///main.rs", |o| o.links = false, cx);
    assert!(links(&editor, cx).is_empty());
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.links = true;
        e.update_options(options, cx);
    });
    assert_eq!(links(&editor, cx), vec![link(0, 0, 12, "https://a.io")]);
}

/// Links each `#N` to an issue, resolving its URL on demand.
struct Issues;
impl LinkProvider for Issues {
    fn provide_links(
        &self,
        context: &ProviderContext,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<DocumentLink>>> {
        let line = context.snapshot.line(0);
        let links = line
            .match_indices('#')
            .map(|(ix, _)| DocumentLink {
                range: PointUtf16::new(0, ix as u32)..PointUtf16::new(0, ix as u32 + 3),
                url: None,
                tooltip: Some("Open issue".into()),
            })
            .collect();
        Task::ready(Ok(links))
    }

    fn resolve_link(
        &self,
        link: &DocumentLink,
        _: &mut App,
    ) -> Option<Task<anyhow::Result<DocumentLink>>> {
        Some(Task::ready(Ok(DocumentLink {
            url: Some(format!("https://issues/{}", link.range.start.column).into()),
            ..link.clone()
        })))
    }
}

#[gpui::test]
fn provider_links_replace_overlapping_ones_and_resolve(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_link_provider(cx, LanguageSelector::All, Issues);
    });
    let (editor, cx) = setup("see #12 and http://x.io/#12", "file:///main.rs", |_| {}, cx);
    // The provider's `#12` in the URL replaces the URL.
    assert_eq!(
        links(&editor, cx),
        vec![
            (PointUtf16::new(0, 4)..PointUtf16::new(0, 7), None),
            (PointUtf16::new(0, 24)..PointUtf16::new(0, 27), None),
        ]
    );
    let opened = opened(&editor, cx);
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(
                0, 5,
            ))],
            cx,
        );
        e.open_link(cx);
    });
    cx.run_until_parked();
    assert_eq!(*opened.borrow(), vec!["https://issues/4".to_string()]);
}

#[gpui::test]
fn opening_links(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        "https://a.io  file:///./notes.md",
        "file:///home/me/src/main.rs",
        |_| {},
        cx,
    );
    let opened = opened(&editor, cx);
    let open_at = |column: u32, cx: &mut VisualTestContext| {
        editor.update(cx, |e, cx| {
            e.set_selections(
                &[gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(
                    0, column,
                ))],
                cx,
            );
            e.open_link(cx);
        });
    };
    open_at(3, cx);
    // Relative file links resolve against the model's directory.
    open_at(20, cx);
    // Nothing to open between them (a link's end still counts, as in
    // Monaco).
    open_at(13, cx);
    assert_eq!(
        *opened.borrow(),
        vec![
            "https://a.io".to_string(),
            "file:///home/me/src/notes.md".to_string()
        ]
    );
}

/// Monaco `ClickLinkGesture`: Cmd/Ctrl+click and the `openLink` middle
/// click open a link on mouse-up, on the row the mouse went down on; a
/// drag or a scroll in between cancels it.
#[gpui::test]
fn links_open_on_mouse_up(cx: &mut TestAppContext) {
    let text = format!("see https://a.io now\n{}", "line\n".repeat(100));
    let (editor, cx) = setup(&text, "file:///main.rs", |_| {}, cx);
    let opened = opened(&editor, cx);
    let draw = |cx: &mut VisualTestContext| {
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        });
    };
    draw(cx);
    let point = |column: u32, cx: &mut VisualTestContext| {
        editor
            .read_with(cx, |e, _| {
                e.scrolled_visible_position(PointUtf16::new(0, column))
            })
            .unwrap()
            + gpui::point(gpui::px(1.), gpui::px(2.))
    };
    let at = point(8, cx);
    let cmd = gpui::Modifiers::secondary_key();
    let left = gpui::MouseButton::Left;
    cx.simulate_mouse_down(at, left, cmd);
    assert!(opened.borrow().is_empty());
    cx.simulate_mouse_up(at, left, cmd);
    assert_eq!(*opened.borrow(), vec!["https://a.io".to_string()]);

    // Pressed, dragged away and released: a selection, not a link.
    let away = point(18, cx);
    cx.simulate_mouse_down(at, left, cmd);
    cx.simulate_mouse_move(away, left, cmd);
    cx.simulate_mouse_up(away, left, cmd);
    assert_eq!(opened.borrow().len(), 1);
    assert!(!editor.read_with(cx, |e, _| e.selections()[0].is_empty()));

    // Scrolling in between cancels it too.
    draw(cx);
    cx.simulate_mouse_down(at, left, cmd);
    editor.update(cx, |e, cx| {
        e.set_scroll_position(gpui::point(gpui::px(0.), gpui::px(5.)), cx)
    });
    cx.simulate_mouse_up(at, left, cmd);
    assert_eq!(opened.borrow().len(), 1);

    // Monaco `mouseMiddleClickAction: openLink`: on mouse-up too.
    editor.update(cx, |e, cx| {
        e.set_scroll_position(gpui::point(gpui::px(0.), gpui::px(0.)), cx);
        let mut options = e.options().clone();
        options.mouse_middle_click_action = gpui_monaco::options::MouseMiddleClickAction::OpenLink;
        e.update_options(options, cx);
    });
    draw(cx);
    let middle = gpui::MouseButton::Middle;
    let none = gpui::Modifiers::default();
    cx.simulate_mouse_down(at, middle, none);
    assert_eq!(opened.borrow().len(), 1);
    cx.simulate_mouse_up(at, middle, none);
    assert_eq!(opened.borrow().len(), 2);
}

/// Links to fixed urls.
struct Fixed(Vec<DocumentLink>);
impl LinkProvider for Fixed {
    fn provide_links(
        &self,
        _: &ProviderContext,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<DocumentLink>>> {
        Task::ready(Ok(self.0.clone()))
    }
}

fn fixed(row: u32, start: u32, end: u32, url: &str) -> Fixed {
    Fixed(vec![DocumentLink {
        range: PointUtf16::new(row, start)..PointUtf16::new(row, end),
        url: Some(url.to_string().into()),
        tooltip: None,
    }])
}

/// Monaco `openLinkOccurrence` hands links to the opener service (with
/// `allowCommands`): with no link opener taking them, web links open in
/// the browser, `command:` links run, and a link into the model selects
/// its fragment; only what nothing opens is reported.
#[gpui::test]
fn detected_links_open_by_default(cx: &mut TestAppContext) {
    let ran = Rc::new(RefCell::new(Vec::new()));
    let sink = ran.clone();
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_link_provider(
            cx,
            LanguageSelector::All,
            fixed(1, 0, 3, "command:test.run?%5B7%5D"),
        );
        let _command = gpui_monaco::register_command(cx, "test.run", move |args, _| {
            sink.borrow_mut().push(args.to_vec());
        });
    });
    let (editor, cx) = setup(
        "https://a.io file:///main.rs#L3,2 file:///other.rs\nrun\nthird line\n",
        "file:///main.rs",
        |_| {},
        cx,
    );
    assert_eq!(links(&editor, cx).len(), 4);
    let reported = reported(&editor, cx);
    let open_at = |row: u32, column: u32, cx: &mut VisualTestContext| {
        editor.update(cx, |e, cx| {
            e.set_selections(
                &[gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(
                    row, column,
                ))],
                cx,
            );
            e.open_link(cx);
        });
        cx.run_until_parked();
    };
    open_at(0, 3, cx);
    assert_eq!(cx.opened_url().as_deref(), Some("https://a.io"));
    open_at(1, 1, cx);
    assert_eq!(*ran.borrow(), vec![vec![serde_json::json!(7)]]);
    open_at(0, 20, cx);
    let selection = editor.read_with(cx, |e, _| e.selection());
    assert_eq!(selection.head, PointUtf16::new(2, 1));
    assert!(reported.borrow().is_empty());
    // Another model, and no editor opener.
    open_at(0, 40, cx);
    assert_eq!(*reported.borrow(), ["file:///other.rs"]);
}

/// Monaco `LinkDetector`: links are recomputed right away when the
/// model's language or the link providers change.
#[gpui::test]
fn links_follow_language_and_provider_changes(cx: &mut TestAppContext) {
    let (editor, cx) = setup("see #12 and https://a.io", "file:///main.rs", |_| {}, cx);
    assert_eq!(links(&editor, cx), vec![link(0, 12, 24, "https://a.io")]);
    let registration = cx.update(|_, cx| {
        register_link_provider(
            cx,
            LanguageSelector::Languages(vec!["rust".into()]),
            fixed(0, 4, 7, "https://issues/12"),
        )
    });
    assert_eq!(links(&editor, cx).len(), 1);
    editor.update(cx, |e, cx| {
        e.model()
            .update(cx, |m, cx| m.set_language(Some("rust".into()), cx))
    });
    assert_eq!(
        links(&editor, cx),
        vec![
            link(0, 4, 7, "https://issues/12"),
            link(0, 12, 24, "https://a.io")
        ]
    );
    cx.update(|_, cx| unregister_provider(cx, registration));
    assert_eq!(links(&editor, cx), vec![link(0, 12, 24, "https://a.io")]);
}

/// Monaco `_executeLinkProvider` is `getLinks`: the built-in URL links
/// and every provider's, the better-ranked provider's replacing the
/// links they overlap, sorted.
#[gpui::test]
fn execute_link_provider_merges_like_the_editor(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let model = cx.new(|_| {
        TextModel::new("x https://a.example y #12 z")
            .with_uri("file:///main.rs")
            .with_language("rust")
    });
    cx.update(|cx| {
        // Better ranked (a language match) than `All`.
        register_link_provider(
            cx,
            LanguageSelector::Languages(vec!["rust".into()]),
            fixed(0, 22, 25, "https://best/12"),
        );
        register_link_provider(
            cx,
            LanguageSelector::All,
            Fixed(vec![
                DocumentLink {
                    range: PointUtf16::new(0, 0)..PointUtf16::new(0, 1),
                    url: Some("https://x".into()),
                    tooltip: None,
                },
                DocumentLink {
                    range: PointUtf16::new(0, 23)..PointUtf16::new(0, 26),
                    url: Some("https://worse/12".into()),
                    tooltip: None,
                },
            ]),
        );
    });
    let task = cx.update(|cx| gpui_monaco::api_commands::execute_link_provider(&model, 0, cx));
    cx.run_until_parked();
    let found = Rc::new(RefCell::new(None));
    let sink = found.clone();
    cx.spawn(async move |_| *sink.borrow_mut() = Some(task.await))
        .detach();
    cx.run_until_parked();
    let links: Vec<_> = found
        .take()
        .expect("links")
        .into_iter()
        .map(|l| (l.range, l.url.map(|u| u.to_string())))
        .collect();
    let at = |start: u32, end: u32| PointUtf16::new(0, start)..PointUtf16::new(0, end);
    assert_eq!(
        links,
        vec![
            (at(0, 1), Some("https://x".to_string())),
            (at(2, 19), Some("https://a.example".to_string())),
            (at(22, 25), Some("https://best/12".to_string())),
        ]
    );
}
