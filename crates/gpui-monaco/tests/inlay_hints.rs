//! Inlay hints (Monaco `contrib/inlayHints`) with test providers.

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, AppContext as _, Entity, Focusable, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent,
    Pixels, Point, Task, TestAppContext, VisualTestContext,
};
use gpui_monaco::options::InlayHintsMode;
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, CodeEditorEvent, HOVER_DELAY, TextModel};

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

/// A `: i32` hint after every `let x`.
#[derive(Clone, Default)]
struct Types {
    requests: Rc<RefCell<Vec<Range<PointUtf16>>>>,
}

impl InlayHintsProvider for Types {
    fn provide_inlay_hints(
        &self,
        context: &ProviderContext,
        range: Range<PointUtf16>,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<InlayHint>>> {
        self.requests.borrow_mut().push(range.clone());
        let mut hints = Vec::new();
        for row in range.start.row..=range.end.row {
            let line = context.snapshot.line(row);
            if let Some(start) = line.find("let ") {
                let end = start + 4 + line[start + 4..].find(' ').unwrap_or(0);
                hints.push(InlayHint {
                    position: p(row, end as u32),
                    label: ": i32".into(),
                    kind: Some(InlayHintKind::Type),
                    ..Default::default()
                });
            }
        }
        Task::ready(Ok(hints))
    }
}

fn setup_with<'a>(
    text: &str,
    register: impl FnOnce(&mut App),
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register(cx);
    });
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    settle(cx);
    (editor, cx)
}

fn setup(types: Types, cx: &mut TestAppContext) -> (Entity<CodeEditor>, &mut VisualTestContext) {
    setup_with(
        "let x = 1;\nlet yy = 2;\n",
        |cx| {
            register_inlay_hints_provider(cx, LanguageSelector::All, types);
        },
        cx,
    )
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
}

/// Draws, lets the debounce pass, and draws again.
fn settle(cx: &mut VisualTestContext) {
    draw(cx);
    cx.executor().advance_clock(Duration::from_millis(300));
    cx.run_until_parked();
    draw(cx);
}

fn hints(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<(PointUtf16, String)> {
    editor.read_with(cx, |e, cx| e.visible_inlay_hints(cx))
}

#[gpui::test]
fn hints_show_inline_and_follow_edits(cx: &mut TestAppContext) {
    let types = Types::default();
    let requests = types.requests.clone();
    let (editor, cx) = setup(types, cx);
    assert_eq!(
        hints(&editor, cx),
        vec![
            (p(0, 5), ": i32".to_string()),
            (p(1, 6), ": i32".to_string())
        ]
    );
    // The request covers the viewport.
    assert_eq!(requests.borrow()[0].start, p(0, 0));
    // They push the text after them right.
    let x_after = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, _| e.scrolled_visible_position(p(0, 6)).unwrap().x)
    };
    let with_hint = x_after(cx);
    // An edit moves them along, then they're requested again.
    let before = requests.borrow().len();
    editor.update(cx, |e, cx| {
        e.execute_edits(vec![(p(0, 0)..p(0, 0), "  ".into())], None, cx);
    });
    assert_eq!(hints(&editor, cx)[0].0, p(0, 7));
    settle(cx);
    assert!(requests.borrow().len() > before);
    assert_eq!(hints(&editor, cx)[0].0, p(0, 7));

    // Off: none, and nothing is asked.
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.inlay_hints.enabled = InlayHintsMode::Off;
        e.update_options(options, cx);
    });
    settle(cx);
    assert!(hints(&editor, cx).is_empty());
    assert!(x_after(cx) < with_hint);
}

#[gpui::test]
fn ctrl_alt_toggles_them(cx: &mut TestAppContext) {
    let (editor, cx) = setup(Types::default(), cx);
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.inlay_hints.enabled = InlayHintsMode::OffUnlessPressed;
        e.update_options(options, cx);
    });
    settle(cx);
    assert!(hints(&editor, cx).is_empty());
    cx.simulate_modifiers_change(Modifiers {
        control: true,
        alt: true,
        ..Default::default()
    });
    assert_eq!(hints(&editor, cx).len(), 2);
    cx.simulate_modifiers_change(Modifiers::default());
    assert!(hints(&editor, cx).is_empty());
}

#[gpui::test]
fn refresh_asks_again(cx: &mut TestAppContext) {
    let types = Types::default();
    let requests = types.requests.clone();
    let (_editor, cx) = setup(types, cx);
    let before = requests.borrow().len();
    cx.update(|_, cx| refresh_inlay_hints(cx));
    settle(cx);
    assert_eq!(requests.borrow().len(), before + 1);
}

/// Monaco `editor.inlayHints.fontSize`: smaller hints take less room, and
/// sizes above the editor's use the editor's.
#[gpui::test]
fn font_size_option(cx: &mut TestAppContext) {
    let (editor, cx) = setup(Types::default(), cx);
    let x_after = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, _| e.scrolled_visible_position(p(0, 6)).unwrap().x)
    };
    let set_size = |size: f32, cx: &mut VisualTestContext| {
        editor.update(cx, |e, cx| {
            let mut options = e.options().clone();
            options.inlay_hints.font_size = size;
            e.update_options(options, cx);
        });
        settle(cx);
    };
    let normal = x_after(cx);
    set_size(6., cx);
    assert_eq!(hints(&editor, cx).len(), 2);
    assert!(x_after(cx) < normal);
    set_size(100., cx);
    assert_eq!(x_after(cx), normal);
    // Too small to use.
    set_size(4., cx);
    assert_eq!(x_after(cx), normal);
}

/// `: Foo` (location: row 1) `<T>` (command) after `let a`, with a hint
/// tooltip, a part tooltip and an insert edit. Resolving adds the tooltip.
#[derive(Clone, Default)]
struct Parts {
    resolves: Rc<Cell<usize>>,
    /// How long resolving takes.
    resolve_delay: Duration,
}

fn label_part(label: &str) -> InlayHintLabelPart {
    InlayHintLabelPart {
        label: label.into(),
        ..Default::default()
    }
}

impl InlayHintsProvider for Parts {
    fn provide_inlay_hints(
        &self,
        _: &ProviderContext,
        _: Range<PointUtf16>,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<InlayHint>>> {
        Task::ready(Ok(vec![InlayHint {
            position: p(0, 5),
            label: vec![
                label_part(": "),
                InlayHintLabelPart {
                    tooltip: Some(MarkdownString::new("**Foo** docs").into()),
                    location: Some(Location {
                        uri: None,
                        range: p(1, 7)..p(1, 10),
                        ..Default::default()
                    }),
                    ..label_part("Foo")
                },
                InlayHintLabelPart {
                    command: Some(Command {
                        id: "test.showT".into(),
                        title: "Show T".into(),
                        arguments: vec![serde_json::json!(1)],
                        ..Default::default()
                    }),
                    ..label_part("<T>")
                },
            ]
            .into(),
            kind: Some(InlayHintKind::Type),
            text_edits: vec![(p(0, 5)..p(0, 5), ": Foo<T>".into())],
            ..Default::default()
        }]))
    }

    fn resolve_inlay_hint(
        &self,
        hint: &InlayHint,
        cx: &mut App,
    ) -> Option<Task<anyhow::Result<InlayHint>>> {
        self.resolves.set(self.resolves.get() + 1);
        let resolved = InlayHint {
            tooltip: Some(MarkdownString::new("the type of `a`").into()),
            ..hint.clone()
        };
        let timer = cx.background_executor().timer(self.resolve_delay);
        Some(cx.background_executor().spawn(async move {
            timer.await;
            Ok(resolved)
        }))
    }
}

fn setup_parts(
    parts: Parts,
    cx: &mut TestAppContext,
) -> (Entity<CodeEditor>, &mut VisualTestContext) {
    setup_with(
        "let a = make();\nstruct Foo;",
        |cx| {
            register_inlay_hints_provider(cx, LanguageSelector::All, parts);
        },
        cx,
    )
}

fn part_point(
    editor: &Entity<CodeEditor>,
    text: &str,
    cx: &mut VisualTestContext,
) -> Point<Pixels> {
    editor
        .read_with(cx, |e, cx| e.inlay_hint_part_point_for_tests(text, cx))
        .expect("part shown")
}

#[gpui::test]
fn label_parts_show_as_one_hint(cx: &mut TestAppContext) {
    let (editor, cx) = setup_parts(Parts::default(), cx);
    assert_eq!(hints(&editor, cx), vec![(p(0, 5), ": Foo<T>".to_string())]);
}

/// Monaco `InlayHintsHover`: the hint's tooltip (after resolving), the
/// part's, and how to follow the part.
#[gpui::test]
fn hovering_a_part_shows_its_tooltips(cx: &mut TestAppContext) {
    // Resolving outlasts the hover delay.
    let parts = Parts {
        resolve_delay: HOVER_DELAY * 2,
        ..Default::default()
    };
    let resolves = parts.resolves.clone();
    let (editor, cx) = setup_parts(parts, cx);
    let position = part_point(&editor, "Foo", cx);
    cx.simulate_mouse_move(position, None, Modifiers::default());
    assert!(!editor.read_with(cx, |e, _| e.is_hover_visible()));
    cx.executor().advance_clock(HOVER_DELAY);
    cx.run_until_parked();
    assert!(!editor.read_with(cx, |e, _| e.is_hover_visible()));
    cx.executor().advance_clock(HOVER_DELAY);
    cx.run_until_parked();
    assert!(editor.read_with(cx, |e, _| e.is_hover_visible()));
    let keys = if cfg!(target_os = "macos") {
        "cmd + click"
    } else {
        "ctrl + click"
    };
    assert_eq!(
        editor.read_with(cx, |e, cx| e.hover_contents_for_tests(cx)),
        [
            "the type of `a`".to_string(),
            "**Foo** docs".to_string(),
            format!("Go to Definition ({keys})"),
            "Double-click to insert".to_string(),
        ]
    );
    // Resolved once, though the mouse moved over it and the hover asked.
    assert_eq!(resolves.get(), 1);

    // The command part offers its command.
    draw(cx);
    let position = part_point(&editor, "<T>", cx);
    cx.simulate_mouse_move(position, None, Modifiers::default());
    cx.executor().advance_clock(HOVER_DELAY);
    cx.run_until_parked();
    let contents = editor.read_with(cx, |e, cx| e.hover_contents_for_tests(cx));
    assert!(contents.contains(&format!("Execute Command ({keys})")));
    assert_eq!(resolves.get(), 1);

    // Off the hint, it goes.
    draw(cx);
    let text = editor
        .read_with(cx, |e, _| e.scrolled_visible_position(p(0, 12)))
        .unwrap();
    cx.simulate_mouse_move(
        gpui::point(text.x + gpui::px(1.), text.y + gpui::px(2.)),
        None,
        Modifiers::default(),
    );
    assert!(!editor.read_with(cx, |e, _| e.is_hover_visible()));
}

/// Monaco's link gesture: Cmd/Ctrl+click on a part with a location goes
/// there; on one with a command runs it.
#[gpui::test]
fn cmd_click_follows_a_part(cx: &mut TestAppContext) {
    let (editor, cx) = setup_parts(Parts::default(), cx);
    let ran = Rc::new(RefCell::new(Vec::new()));
    let sink = ran.clone();
    cx.update(|_, cx| {
        gpui_monaco::register_command(cx, "test.showT", move |args, _| {
            sink.borrow_mut().push(args.to_vec())
        })
    });
    let position = part_point(&editor, "Foo", cx);
    cx.simulate_mouse_move(position, None, Modifiers::secondary_key());
    cx.simulate_click(position, Modifiers::secondary_key());
    assert_eq!(editor.read_with(cx, |e, _| e.selections()[0].head), p(1, 7));

    draw(cx);
    let position = part_point(&editor, "<T>", cx);
    cx.simulate_click(position, Modifiers::secondary_key());
    assert_eq!(*ran.borrow(), vec![vec![serde_json::json!(1)]]);

    // Without a registered command, the host is asked to run it.
    cx.update(|_, cx| gpui_monaco::unregister_command(cx, "test.showT"));
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    cx.update(|_, cx| {
        cx.subscribe(&editor, move |_, event: &CodeEditorEvent, _| {
            if let CodeEditorEvent::ExecuteCommand(command) = event {
                sink.borrow_mut().push(command.id.to_string());
            }
        })
        .detach();
    });
    cx.simulate_click(position, Modifiers::secondary_key());
    assert_eq!(*events.borrow(), ["test.showT"]);

    // A plain click just places the cursor.
    let position = part_point(&editor, "Foo", cx);
    cx.simulate_click(position, Modifiers::default());
    assert_ne!(editor.read_with(cx, |e, _| e.selections()[0].head), p(1, 7));
}

/// Monaco `_installDblClickGesture`: double-clicking a hint applies its
/// text edits.
#[gpui::test]
fn double_click_inserts_the_hint(cx: &mut TestAppContext) {
    let (editor, cx) = setup_parts(Parts::default(), cx);
    let position = part_point(&editor, ": ", cx);
    cx.simulate_event(MouseDownEvent {
        position,
        button: MouseButton::Left,
        modifiers: Modifiers::default(),
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position,
        button: MouseButton::Left,
        modifiers: Modifiers::default(),
        click_count: 2,
    });
    cx.run_until_parked();
    let text = editor.read_with(cx, |e, cx| e.model().read(cx).text());
    assert_eq!(text, "let a: Foo<T> = make();\nstruct Foo;");
}
