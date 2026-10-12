//! Code lenses (Monaco `contrib/codelens`) with a mock provider.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gpui::{App, AppContext as _, Entity, Focusable, Task, TestAppContext, VisualTestContext};
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, CodeEditorEvent, TextModel, register_command};

/// A lens above every `fn` line: resolved later for the second one.
struct Functions {
    requests: Rc<Cell<usize>>,
}

impl CodeLensProvider for Functions {
    fn provide_code_lenses(
        &self,
        context: &ProviderContext,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<CodeLens>>> {
        self.requests.set(self.requests.get() + 1);
        let rope = context.snapshot.rope().to_string();
        let lenses = rope
            .lines()
            .enumerate()
            .filter(|(_, line)| line.trim_start().starts_with("fn "))
            .enumerate()
            .map(|(ix, (row, _))| CodeLens {
                range: PointUtf16::new(row as u32, 0)..PointUtf16::new(row as u32, 2),
                command: (ix == 0).then(|| Command {
                    id: "test.references".into(),
                    title: "2 references".into(),
                    arguments: vec![serde_json::json!(row)],
                    ..Default::default()
                }),
                ..Default::default()
            })
            .collect();
        Task::ready(Ok(lenses))
    }

    fn resolve_code_lens(
        &self,
        _: &ProviderContext,
        lens: &CodeLens,
        _: &mut App,
    ) -> Option<Task<anyhow::Result<CodeLens>>> {
        Some(Task::ready(Ok(CodeLens {
            command: Some(Command {
                id: "test.unregistered".into(),
                title: "Run test".into(),
                arguments: Vec::new(),
                ..Default::default()
            }),
            ..lens.clone()
        })))
    }
}

fn setup<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (
    Entity<CodeEditor>,
    &'a mut VisualTestContext,
    Rc<Cell<usize>>,
) {
    let requests = Rc::new(Cell::new(0));
    let provider = Functions {
        requests: requests.clone(),
    };
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_code_lens_provider(cx, LanguageSelector::All, provider);
    });
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx, requests)
}

fn lenses(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
) -> Vec<(u32, Vec<Option<String>>)> {
    cx.run_until_parked();
    editor.read_with(cx, |e, _| e.code_lenses_for_tests())
}

#[gpui::test]
fn lenses_show_above_their_lines_and_resolve(cx: &mut TestAppContext) {
    let (editor, cx, requests) = setup("fn a() {}\n\nfn b() {}\n", cx);
    assert_eq!(
        lenses(&editor, cx),
        vec![
            (0, vec![Some("2 references".into())]),
            // Resolved once visible.
            (2, vec![Some("Run test".into())]),
        ]
    );
    assert_eq!(requests.get(), 1);
}

#[gpui::test]
fn clicking_runs_the_command(cx: &mut TestAppContext) {
    let (editor, cx, _) = setup("fn a() {}\nfn b() {}\n", cx);
    let ran = Rc::new(RefCell::new(Vec::new()));
    let unhandled = Rc::new(RefCell::new(Vec::new()));
    cx.update(|_, cx| {
        let ran = ran.clone();
        let _ = register_command(cx, "test.references", move |args, _| {
            ran.borrow_mut().push(args.to_vec())
        });
    });
    let sink = unhandled.clone();
    cx.update(|_, cx| {
        cx.subscribe(&editor, move |_, event, _| {
            if let CodeEditorEvent::ExecuteCommand(command) = event {
                sink.borrow_mut().push(command.id.to_string());
            }
        })
        .detach()
    });
    lenses(&editor, cx);
    editor.update(cx, |e, cx| e.click_code_lens_for_tests(0, 0, cx));
    assert_eq!(*ran.borrow(), vec![vec![serde_json::json!(0)]]);
    // No handler: the host gets it.
    editor.update(cx, |e, cx| e.click_code_lens_for_tests(1, 0, cx));
    assert_eq!(*unhandled.borrow(), vec!["test.unregistered".to_string()]);
}

#[gpui::test]
fn refreshed_after_edits(cx: &mut TestAppContext) {
    let (editor, cx, requests) = setup("fn a() {}\n", cx);
    assert_eq!(lenses(&editor, cx).len(), 1);
    editor.update(cx, |e, cx| {
        e.model()
            .update(cx, |m, cx| m.edit([(0..0, "fn z() {}\n")], cx).unwrap())
    });
    // The zones follow the edit until new lenses arrive after a pause.
    cx.run_until_parked();
    assert_eq!(requests.get(), 1);
    cx.executor().advance_clock(Duration::from_millis(500));
    assert_eq!(lenses(&editor, cx).len(), 2);
    assert_eq!(requests.get(), 2);
}

/// Monaco `CodeLensContribution` listens to
/// `codeLensProvider.onDidChange`: providers registered or removed later
/// are asked again (after the debounce) without an edit, and with none
/// left the lenses go.
#[gpui::test]
fn lenses_follow_providers_registered_and_removed_later(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new("fn a() {}\n\nfn b() {}\n"));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    assert!(lenses(&editor, cx).is_empty());
    let requests = Rc::new(Cell::new(0));
    let provider = |requests: &Rc<Cell<usize>>| Functions {
        requests: requests.clone(),
    };
    let a = cx.update(|_, cx| {
        register_code_lens_provider(cx, LanguageSelector::All, provider(&requests))
    });
    assert_eq!(lenses(&editor, cx).len(), 2);
    assert_eq!(requests.get(), 1);
    let b = cx.update(|_, cx| {
        register_code_lens_provider(cx, LanguageSelector::All, provider(&requests))
    });
    cx.executor().advance_clock(Duration::from_millis(500));
    // Both providers' lenses, two per line.
    assert_eq!(
        lenses(&editor, cx),
        vec![
            (
                0,
                vec![Some("2 references".into()), Some("2 references".into())]
            ),
            (2, vec![Some("Run test".into()), Some("Run test".into())]),
        ]
    );
    cx.update(|_, cx| unregister_provider(cx, a));
    cx.executor().advance_clock(Duration::from_millis(500));
    assert_eq!(
        lenses(&editor, cx),
        vec![
            (0, vec![Some("2 references".into())]),
            (2, vec![Some("Run test".into())]),
        ]
    );
    cx.update(|_, cx| unregister_provider(cx, b));
    assert!(lenses(&editor, cx).is_empty());
}

#[gpui::test]
fn code_lens_option(cx: &mut TestAppContext) {
    let (editor, cx, _) = setup("fn a() {}\n", cx);
    assert_eq!(lenses(&editor, cx).len(), 1);
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.code_lens = false;
        e.update_options(options, cx);
    });
    assert!(lenses(&editor, cx).is_empty());
}

#[gpui::test]
fn show_lenses_in_current_line(cx: &mut TestAppContext) {
    let (editor, cx, _) = setup("fn a() {}\nlet x;\n", cx);
    let ran = Rc::new(RefCell::new(Vec::new()));
    cx.update(|_, cx| {
        let ran = ran.clone();
        let _ = register_command(cx, "test.references", move |args, _| {
            ran.borrow_mut().push(args.to_vec())
        });
    });
    lenses(&editor, cx);
    let run = |editor: &Entity<CodeEditor>, cx: &mut VisualTestContext| {
        cx.update(|window, cx| {
            editor.update(cx, |e, cx| {
                e.run_action("codelens.showLensesInCurrentLine", window, cx)
            })
        });
        cx.run_until_parked();
    };
    // A line without lenses: nothing shows.
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(
                1, 0,
            ))],
            cx,
        )
    });
    run(&editor, cx);
    assert!(!editor.read_with(cx, |e, _| e.is_quick_access_open()));
    // The lens's line: its commands in the quick input; choosing runs one.
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(
                0, 3,
            ))],
            cx,
        )
    });
    run(&editor, cx);
    assert_eq!(
        editor.read_with(cx, |e, cx| e.quick_access_rows_for_tests(cx)),
        ["> 2 references"]
    );
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(*ran.borrow(), vec![vec![serde_json::json!(0)]]);
}

/// A lens above every `fn` line, resolved after `delay`; the lens above a
/// line with `fail` fails to resolve.
struct Slow {
    delay: Duration,
}

impl CodeLensProvider for Slow {
    fn provide_code_lenses(
        &self,
        context: &ProviderContext,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<CodeLens>>> {
        let rope = context.snapshot.rope().to_string();
        let lenses = rope
            .lines()
            .enumerate()
            .filter(|(_, line)| line.contains("fn "))
            .map(|(row, line)| {
                let column = line.find("fn ").unwrap() as u32;
                CodeLens {
                    range: PointUtf16::new(row as u32, column)
                        ..PointUtf16::new(row as u32, column + 2),
                    ..Default::default()
                }
            })
            .collect();
        Task::ready(Ok(lenses))
    }

    fn resolve_code_lens(
        &self,
        context: &ProviderContext,
        lens: &CodeLens,
        cx: &mut App,
    ) -> Option<Task<anyhow::Result<CodeLens>>> {
        let fail = context.snapshot.line(lens.range.start.row).contains("fail");
        let lens = lens.clone();
        let timer = cx.background_executor().timer(self.delay);
        Some(cx.background_executor().spawn(async move {
            timer.await;
            anyhow::ensure!(!fail, "no references");
            Ok(CodeLens {
                command: Some(Command {
                    id: "test.references".into(),
                    title: "$(references) 3 references".into(),
                    tooltip: Some("Show references".into()),
                    ..Default::default()
                }),
                ..lens
            })
        }))
    }
}

fn setup_slow<'a>(
    text: &str,
    delay: Duration,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_code_lens_provider(cx, LanguageSelector::All, Slow { delay });
    });
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    draw(cx);
    (editor, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
}

fn lines(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
) -> Vec<(u32, String, bool, gpui::Pixels)> {
    editor.read_with(cx, |e, cx| e.code_lens_lines_for_tests(cx))
}

/// Monaco `CodeLensWidget`: lenses waiting to resolve leave their zone
/// empty, then fade in once resolved; "no commands" shows only when
/// resolving gave none.
#[gpui::test]
fn unresolved_lenses_leave_their_zone_empty(cx: &mut TestAppContext) {
    let (editor, cx) = setup_slow("fn a() {}\nfn fail() {}\n", Duration::from_millis(100), cx);
    let shown = lines(&editor, cx);
    assert_eq!(shown.len(), 2);
    assert!(
        shown
            .iter()
            .all(|(_, text, fading, _)| text.is_empty() && !fading)
    );
    cx.executor().advance_clock(Duration::from_millis(150));
    draw(cx);
    let shown = lines(&editor, cx);
    assert_eq!(shown[0].1, "$(references) 3 references");
    assert!(shown[0].2, "resolved lenses fade in");
    assert_eq!(shown[1].1, "no commands");
    assert!(!shown[1].2);
}

/// Monaco `_updateLensStyle`: the lenses' font size and zone height follow
/// the editor's font size and zoom, and their color the theme.
#[gpui::test]
fn lenses_follow_font_zoom_and_theme(cx: &mut TestAppContext) {
    let (editor, cx) = setup_slow("\nfn a() {}\n", Duration::ZERO, cx);
    draw(cx);
    let style = |cx: &mut VisualTestContext| {
        editor
            .read_with(cx, |e, cx| e.code_lens_style_for_tests(1, cx))
            .unwrap()
    };
    let height = |cx: &mut VisualTestContext| lines(&editor, cx)[0].3;
    let line_top = |cx: &mut VisualTestContext| {
        editor
            .read_with(cx, |e, _| {
                e.scrolled_visible_position(PointUtf16::new(1, 0))
            })
            .unwrap()
            .y
    };
    let (_, size, color) = style(cx);
    let zone = height(cx);
    let top = line_top(cx);
    // Monaco `_getLayoutInfo`: 90% of the editor's font size, and at least
    // 1.3 times that tall.
    let editor_size = editor.read_with(cx, |e, _| f32::from(e.options().font_size));
    assert_eq!(size, gpui::px((editor_size * 0.9).floor()));

    cx.update(|_, cx| gpui_monaco::set_editor_zoom_level(3., cx));
    draw(cx);
    let zoomed = style(cx).1;
    assert_eq!(zoomed, gpui::px((editor_size * 1.3 * 0.9).floor()));
    assert!(height(cx) > zone);
    // The zone is laid out at its new height: the line below moves down
    // by more than the first line grew.
    assert!(line_top(cx) > top);

    cx.update(|_, cx| gpui_monaco::set_editor_zoom_level(0., cx));
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.code_lens_font_size = 20.;
        e.update_options(options, cx);
    });
    draw(cx);
    assert_eq!(style(cx).1, gpui::px(20.));

    editor.update(cx, |e, cx| {
        e.set_theme(std::sync::Arc::new(gpui_monaco::Theme::hc_black()), cx)
    });
    draw(cx);
    assert_ne!(style(cx).2, color);
}

/// Monaco puts the lenses at the line's first non-whitespace column: tabs
/// count as tab stops, and the lenses scroll horizontally with the text.
#[gpui::test]
fn lenses_line_up_with_the_text(cx: &mut TestAppContext) {
    let long = "x".repeat(400);
    let (editor, cx) = setup_slow(
        &format!("impl A {{\n\tfn a() {{}}\n\t\t  fn b() {{}}\n    // {long}\n}}\n"),
        Duration::ZERO,
        cx,
    );
    draw(cx);
    let lens_x = |row: u32, cx: &mut VisualTestContext| {
        editor
            .read_with(cx, |e, cx| e.code_lens_style_for_tests(row, cx))
            .unwrap()
            .0
            .x
    };
    let text_x = |row: u32, column: u32, cx: &mut VisualTestContext| {
        editor
            .read_with(cx, |e, _| {
                e.scrolled_visible_position(PointUtf16::new(row, column))
            })
            .unwrap()
            .x
    };
    assert_eq!(lens_x(1, cx), text_x(1, 1, cx));
    assert_eq!(lens_x(2, cx), text_x(2, 4, cx));
    // A tab is wider than a space.
    assert!(lens_x(1, cx) - text_x(1, 0, cx) > gpui::px(20.));
    let before = lens_x(1, cx);
    editor.update(cx, |e, cx| {
        e.set_scroll_left(gpui::px(50.), gpui_monaco::ScrollType::Immediate, cx)
    });
    draw(cx);
    let near = |a: gpui::Pixels, b: gpui::Pixels| (a - b).abs() < gpui::px(0.01);
    assert!(near(lens_x(1, cx), before - gpui::px(50.)));
    assert!(near(lens_x(1, cx), text_x(1, 1, cx)));
}

/// Monaco `_detachModel` drops the old view's zones; `CodeLensContribution`
/// fetches the new model's lenses and gives them zones again.
#[gpui::test]
fn lenses_come_back_after_set_model(cx: &mut TestAppContext) {
    let (editor, cx, _) = setup("fn a() {}\n", cx);
    draw(cx);
    // The height of the zones above a row.
    let zones_above = |row: u32, editor: &Entity<CodeEditor>, cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, _| {
            e.top_for_line_number(row, false).unwrap() - e.top_for_line_number(row, true).unwrap()
        })
    };
    let lens_height = zones_above(0, &editor, cx);
    assert!(lens_height > gpui::px(0.));
    editor.update(cx, |e, cx| {
        let model = cx.new(|_| TextModel::new("\nfn b() {}\n"));
        e.set_model(model, cx);
    });
    draw(cx);
    assert_eq!(
        lenses(&editor, cx),
        vec![(1, vec![Some("2 references".into())])]
    );
    draw(cx);
    assert_eq!(zones_above(0, &editor, cx), gpui::px(0.));
    assert_eq!(zones_above(1, &editor, cx), lens_height);
}

/// Monaco `_renderCodeLensSymbols` merges new lenses into the shown ones by
/// line: after an edit, a line that still has lenses keeps its zone and its
/// titles until the new lenses resolve, without blanking or fading in
/// again; lines that lost theirs drop their zone and new ones get one.
#[gpui::test]
fn lenses_keep_their_titles_across_edits(cx: &mut TestAppContext) {
    let (editor, cx) = setup_slow("fn a() {}\n\nfn b() {}\n", Duration::from_millis(100), cx);
    cx.executor().advance_clock(Duration::from_millis(150));
    draw(cx);
    let shown = lines(&editor, cx);
    assert_eq!(shown.len(), 2);
    assert!(shown.iter().all(|(_, text, _, _)| !text.is_empty()));
    // Past Monaco's 0.1s `fadein`.
    std::thread::sleep(Duration::from_millis(120));
    draw(cx);
    assert!(lines(&editor, cx).iter().all(|(_, _, fading, _)| !fading));

    // Type on the first lens's line, and turn the blank line into a third.
    editor.update(cx, |e, cx| {
        e.model().update(cx, |m, cx| {
            m.edit([(9..9, " "), (10..10, "fn c() {}")], cx).unwrap()
        })
    });
    draw(cx);
    // New lenses after the pause: the kept lines show their old titles
    // while the new ones resolve, and the new line waits empty.
    cx.executor().advance_clock(Duration::from_millis(300));
    draw(cx);
    let shown = lines(&editor, cx);
    assert_eq!(
        shown
            .iter()
            .map(|(row, text, fading, _)| (*row, text.as_str(), *fading))
            .collect::<Vec<_>>(),
        vec![
            (0, "$(references) 3 references", false),
            (1, "", false),
            (2, "$(references) 3 references", false),
        ]
    );
    assert_eq!(
        editor.read_with(cx, |e, _| e.code_lenses_for_tests()),
        vec![(0, vec![None]), (1, vec![None]), (2, vec![None])],
        "the new lenses are still resolving"
    );
    cx.executor().advance_clock(Duration::from_millis(150));
    draw(cx);
    let shown = lines(&editor, cx);
    assert!(shown.iter().all(|(_, text, _, _)| !text.is_empty()));
    // Only the zone that was empty fades in.
    assert_eq!(
        shown
            .iter()
            .map(|(_, _, fading, _)| *fading)
            .collect::<Vec<_>>(),
        vec![false, true, false]
    );

    // Deleting a lens's line drops its zone.
    editor.update(cx, |e, cx| {
        e.model()
            .update(cx, |m, cx| m.edit([(11..21, "")], cx).unwrap())
    });
    cx.executor().advance_clock(Duration::from_millis(400));
    draw(cx);
    cx.executor().advance_clock(Duration::from_millis(150));
    draw(cx);
    assert_eq!(
        lines(&editor, cx)
            .iter()
            .map(|(row, text, _, _)| (*row, text.clone()))
            .collect::<Vec<_>>(),
        vec![
            (0, "$(references) 3 references".to_string()),
            (1, "$(references) 3 references".to_string()),
        ]
    );
}
