//! The hover widget's Monaco fidelity (#247): the problem hover's status
//! bar and body, the `hoverHighlight` decoration, screen reader
//! announcements, full-edge resizing and the full-width row separators.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, AppContext as _, Entity, Focusable as _, Modifiers, Task, TestAppContext,
    VisualTestContext, px,
};
use gpui_monaco::cursor::Selection;
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{
    CodeEditor, CodeEditorEvent, EditorOptions, Marker, MarkerSeverity, RelatedInformation,
    TextModel, Theme,
};

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

/// Offers `actions` after `delay`.
#[derive(Clone, Default)]
struct Fixes {
    actions: Rc<RefCell<Vec<CodeAction>>>,
    delay: Option<Duration>,
}

impl CodeActionProvider for Fixes {
    fn provide_code_actions(
        &self,
        _: &ProviderContext,
        _: Range<PointUtf16>,
        _: &CodeActionContext,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<CodeAction>>> {
        let actions = self.actions.borrow().clone();
        match self.delay {
            None => Task::ready(Ok(actions)),
            Some(delay) => {
                let timer = cx.background_executor().timer(delay);
                cx.background_executor().spawn(async move {
                    timer.await;
                    Ok(actions)
                })
            }
        }
    }
}

/// A hover with `range` for every position.
struct RangedHover(Option<Range<PointUtf16>>);
impl HoverProvider for RangedHover {
    fn provide_hover(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<Hover>>> {
        Task::ready(Ok(Some(Hover {
            contents: vec!["docs".into()],
            range: self.0.clone(),
            ..Default::default()
        })))
    }
}

fn quick_fix(title: &str) -> CodeAction {
    CodeAction {
        title: title.to_string().into(),
        kind: Some(CodeActionKind::QUICK_FIX.into()),
        ..Default::default()
    }
}

fn setup<'a>(
    text: &str,
    options: impl FnOnce(&mut EditorOptions),
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_uri("file:///src/main.rs"));
        CodeEditor::new(model, window, cx)
    });
    editor.update(cx, |editor, cx| {
        let mut opts = editor.options().clone();
        options(&mut opts);
        editor.update_options(opts, cx);
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    paint(cx);
    (editor, cx)
}

fn paint(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn rest(cx: &mut VisualTestContext, ms: u64) {
    cx.executor().advance_clock(Duration::from_millis(ms));
    cx.run_until_parked();
    paint(cx);
}

fn marker(severity: MarkerSeverity, range: Range<PointUtf16>) -> Marker {
    Marker {
        severity,
        message: format!("{severity:?} here").into(),
        range,
        ..Default::default()
    }
}

fn set_markers(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, markers: Vec<Marker>) {
    editor.update(cx, |e, cx| {
        e.model()
            .update(cx, |m, cx| m.set_markers("test", markers, cx))
    });
}

/// Shows the hover at `at` from the keyboard, without waiting.
fn show_at(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, at: PointUtf16) {
    editor.update(cx, |e, cx| {
        e.hide_hover(cx);
        e.set_selections(&[Selection::collapsed(at)], cx);
        e.show_hover_at_cursor(cx);
    });
}

fn status_bar(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<String> {
    editor.read_with(cx, |e, cx| e.hover_status_bar_for_tests(cx))
}

fn labels(status: &[String]) -> Vec<&str> {
    // Strip the platform's keybinding.
    status
        .iter()
        .map(|s| s.split(" (").next().unwrap_or(s))
        .collect()
}

#[gpui::test]
fn the_status_bar_offers_view_problem_and_says_when_there_are_no_fixes(cx: &mut TestAppContext) {
    let (editor, cx) = setup("let x = 1;\n", |_| {}, cx);
    set_markers(
        &editor,
        cx,
        vec![marker(MarkerSeverity::Error, p(0, 4)..p(0, 5))],
    );
    show_at(&editor, cx, p(0, 4));
    cx.run_until_parked();
    // No provider has quick fixes for it (Monaco's "No quick fixes
    // available"), where Quick Fix used to show whenever a provider was
    // registered.
    let status = status_bar(&editor, cx);
    assert_eq!(
        labels(&status),
        ["View Problem", "No quick fixes available"]
    );
    assert!(status[0].ends_with(if cfg!(target_os = "macos") {
        "(⌥F8)"
    } else {
        "(Alt+F8)"
    }));
    // "View Problem" hides the hover and opens the marker zone at it.
    assert!(editor.update(cx, |e, cx| {
        e.run_hover_status_action_for_tests("View Problem", cx)
    }));
    assert!(!editor.read_with(cx, |e, _| e.is_hover_visible()));
    assert_eq!(editor.read_with(cx, |e, _| e.selections()[0].head), p(0, 4));
    assert!(editor.update(cx, |e, cx| e.close_marker_zone(cx)));
}

/// Monaco looks the status bar's keys up (`lookupKeybinding`): a host's
/// binding for Go to Next Problem shows instead of the default.
#[gpui::test]
fn the_status_bar_shows_the_hosts_keybinding(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let _rule = cx.update(|cx| {
        gpui_monaco::add_keybinding_rule(
            cx,
            gpui_monaco::KeybindingRule::new("f9", "editor.action.marker.next"),
        )
        .unwrap()
    });
    let (editor, cx) = setup("let x = 1;\n", |_| {}, cx);
    set_markers(
        &editor,
        cx,
        vec![marker(MarkerSeverity::Error, p(0, 4)..p(0, 5))],
    );
    show_at(&editor, cx, p(0, 4));
    cx.run_until_parked();
    paint(cx);
    let status = status_bar(&editor, cx);
    assert_eq!(status[0], "View Problem (F9)");
}

#[gpui::test]
fn quick_fix_lists_the_markers_quick_fixes(cx: &mut TestAppContext) {
    let fixes = Fixes::default();
    let actions = fixes.actions.clone();
    *actions.borrow_mut() = vec![
        quick_fix("Add semicolon"),
        CodeAction {
            title: "Extract function".into(),
            kind: Some(CodeActionKind::REFACTOR.into()),
            ..Default::default()
        },
    ];
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_code_action_provider(cx, LanguageSelector::All, fixes);
    });
    let (editor, cx) = setup("let x = 1\n", |_| {}, cx);
    set_markers(
        &editor,
        cx,
        vec![marker(MarkerSeverity::Error, p(0, 8)..p(0, 9))],
    );
    show_at(&editor, cx, p(0, 8));
    cx.run_until_parked();
    assert_eq!(
        labels(&status_bar(&editor, cx)),
        ["View Problem", "Quick Fix..."]
    );
    // It lists the marker's quick fixes, not the refactoring.
    assert!(editor.update(cx, |e, cx| {
        e.run_hover_status_action_for_tests("Quick Fix", cx)
    }));
    paint(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_hover_visible()));
    assert!(editor.read_with(cx, |e, _| e.is_code_action_menu_visible()));
    let rows = editor.read_with(cx, |e, cx| e.code_action_menu_rows(cx));
    assert!(
        rows.iter().any(|row| row.contains("Add semicolon")),
        "{rows:?}"
    );
    assert!(!rows.iter().any(|row| row.contains("Extract")), "{rows:?}");

    // Only a refactoring: no quick fixes for the marker.
    editor.update(cx, |e, cx| e.hide_code_action_menu(cx));
    actions.borrow_mut().remove(0);
    show_at(&editor, cx, p(0, 8));
    cx.run_until_parked();
    assert_eq!(
        labels(&status_bar(&editor, cx)),
        ["View Problem", "No quick fixes available"]
    );
}

#[gpui::test]
fn checking_for_quick_fixes_and_remembering_none(cx: &mut TestAppContext) {
    let fixes = Fixes {
        delay: Some(Duration::from_millis(1000)),
        ..Default::default()
    };
    let actions = fixes.actions.clone();
    *actions.borrow_mut() = vec![quick_fix("Fix it")];
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_code_action_provider(cx, LanguageSelector::All, fixes);
    });
    let (editor, cx) = setup("let x = 1;\nlet y = 2;\n", |_| {}, cx);
    set_markers(
        &editor,
        cx,
        vec![
            marker(MarkerSeverity::Warning, p(0, 4)..p(0, 5)),
            marker(MarkerSeverity::Warning, p(1, 4)..p(1, 5)),
        ],
    );
    show_at(&editor, cx, p(0, 4));
    cx.run_until_parked();
    // A blank placeholder first, so the bar doesn't flicker.
    assert_eq!(labels(&status_bar(&editor, cx)), ["View Problem", "\u{a0}"]);
    rest(cx, 250);
    assert_eq!(
        labels(&status_bar(&editor, cx)),
        ["View Problem", "Checking for quick fixes..."]
    );
    rest(cx, 1000);
    assert_eq!(
        labels(&status_bar(&editor, cx)),
        ["View Problem", "Quick Fix..."]
    );

    // A marker known to have none says so right away.
    actions.borrow_mut().clear();
    show_at(&editor, cx, p(1, 4));
    rest(cx, 1250);
    assert_eq!(
        labels(&status_bar(&editor, cx)),
        ["View Problem", "No quick fixes available"]
    );
    show_at(&editor, cx, p(1, 4));
    rest(cx, 250);
    assert_eq!(
        labels(&status_bar(&editor, cx)),
        ["View Problem", "No quick fixes available"]
    );
    // Another marker is asked about afresh.
    show_at(&editor, cx, p(0, 4));
    rest(cx, 250);
    assert_eq!(
        labels(&status_bar(&editor, cx)),
        ["View Problem", "Checking for quick fixes..."]
    );
}

#[gpui::test]
fn read_only_editors_and_hints(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        "let x = 1;\n",
        |o| {
            o.read_only = true;
            // Monaco hides markers in read-only editors by default.
            o.render_validation_decorations = gpui_monaco::options::RenderValidationDecorations::On;
        },
        cx,
    );
    set_markers(
        &editor,
        cx,
        vec![marker(MarkerSeverity::Error, p(0, 4)..p(0, 5))],
    );
    show_at(&editor, cx, p(0, 4));
    rest(cx, 500);
    // No Quick Fix in a read-only editor.
    assert_eq!(labels(&status_bar(&editor, cx)), ["View Problem"]);
    // No View Problem for hints, and so no status bar here.
    set_markers(
        &editor,
        cx,
        vec![marker(MarkerSeverity::Hint, p(0, 4)..p(0, 5))],
    );
    show_at(&editor, cx, p(0, 4));
    rest(cx, 500);
    assert_eq!(
        editor.read_with(cx, |e, cx| e.hover_markers_for_tests(cx).len()),
        1
    );
    assert!(status_bar(&editor, cx).is_empty());
    let status_bar_bounds = cx.debug_bounds("hover-status-bar");
    assert!(status_bar_bounds.is_none());
    // A writable editor still offers the quick fix entry for a hint.
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.read_only = false;
        e.update_options(options, cx);
    });
    show_at(&editor, cx, p(0, 4));
    rest(cx, 0);
    assert_eq!(
        labels(&status_bar(&editor, cx)),
        ["No quick fixes available"]
    );
}

#[gpui::test]
fn the_status_bar_is_for_the_most_severe_marker(cx: &mut TestAppContext) {
    let (editor, cx) = setup("let value = 1;\n", |_| {}, cx);
    set_markers(
        &editor,
        cx,
        vec![
            marker(MarkerSeverity::Warning, p(0, 4)..p(0, 9)),
            marker(MarkerSeverity::Error, p(0, 6)..p(0, 9)),
        ],
    );
    show_at(&editor, cx, p(0, 7));
    cx.run_until_parked();
    assert_eq!(
        editor.read_with(cx, |e, cx| e.hover_markers_for_tests(cx).len()),
        2
    );
    assert!(editor.update(cx, |e, cx| {
        e.run_hover_status_action_for_tests("View Problem", cx)
    }));
    // The error's marker zone.
    assert_eq!(editor.read_with(cx, |e, _| e.selections()[0].head), p(0, 6));
}

#[gpui::test]
fn an_ai_quick_fix_is_offered_directly(cx: &mut TestAppContext) {
    let fixes = Fixes::default();
    *fixes.actions.borrow_mut() = vec![CodeAction {
        title: "Fix with AI".into(),
        kind: Some(CodeActionKind::QUICK_FIX.into()),
        is_ai: true,
        edit: Some(WorkspaceEdit {
            edits: vec![(None, vec![(p(0, 9)..p(0, 9), ";".to_string())])],
            ..Default::default()
        }),
        ..Default::default()
    }];
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_code_action_provider(cx, LanguageSelector::All, fixes);
    });
    let (editor, cx) = setup("let x = 1\n", |_| {}, cx);
    set_markers(
        &editor,
        cx,
        vec![marker(MarkerSeverity::Error, p(0, 8)..p(0, 9))],
    );
    show_at(&editor, cx, p(0, 8));
    cx.run_until_parked();
    assert_eq!(
        labels(&status_bar(&editor, cx)),
        ["View Problem", "Quick Fix...", "Fix with AI"]
    );
    assert!(editor.update(cx, |e, cx| {
        e.run_hover_status_action_for_tests("Fix with AI", cx)
    }));
    cx.run_until_parked();
    assert_eq!(
        editor.read_with(cx, |e, cx| e.model().read(cx).text()),
        "let x = 1;\n"
    );
}

#[gpui::test]
fn code_links_and_related_information_links(cx: &mut TestAppContext) {
    let (editor, cx) = setup("fn a() {}\nfn b() { a(1) }\n", |_| {}, cx);
    let opened = Rc::new(RefCell::new(Vec::new()));
    let sink = opened.clone();
    cx.update(|_, cx| {
        cx.subscribe(&editor, move |_, event, _| match event {
            CodeEditorEvent::OpenLink(url) => sink.borrow_mut().push(url.to_string()),
            CodeEditorEvent::OpenLocation(location) => sink
                .borrow_mut()
                .push(format!("{:?}", location.uri.as_deref())),
            _ => {}
        })
        .detach()
    });
    set_markers(
        &editor,
        cx,
        vec![Marker {
            source: Some("rustc".into()),
            code: Some("E0061".into()),
            code_target: Some("https://doc.rust-lang.org/error_codes/E0061.html".into()),
            related_information: vec![
                RelatedInformation {
                    uri: Some("file:///src/lib.rs".into()),
                    range: p(2, 3)..p(2, 4),
                    message: "defined here".into(),
                },
                RelatedInformation {
                    uri: None,
                    range: p(0, 3)..p(0, 4),
                    message: "and here".into(),
                },
            ],
            ..marker(MarkerSeverity::Error, p(1, 9)..p(1, 13))
        }],
    );
    show_at(&editor, cx, p(1, 10));
    cx.run_until_parked();
    paint(cx);
    // Monaco's copy and accessible content: the message, then
    // `file(line, col): message` per related information.
    let content = editor
        .read_with(cx, |e, cx| e.hover_accessible_content(cx))
        .unwrap();
    assert!(
        content.starts_with("Error here\nlib.rs(3, 4): defined here\nmain.rs(1, 4): and here"),
        "{content}"
    );
    // The code is a link to its target.
    let code = cx.debug_bounds("marker-code-link").expect("code link");
    cx.simulate_click(code.center(), Modifiers::default());
    cx.run_until_parked();
    assert_eq!(
        *opened.borrow(),
        ["https://doc.rust-lang.org/error_codes/E0061.html"]
    );
    assert!(editor.read_with(cx, |e, _| e.is_hover_visible()));
    // Only `file(line, col): ` is the related information's link: a click
    // on the message does nothing.
    let link = cx.debug_bounds("hover-related-0").expect("related link");
    let after_link = gpui::point(link.right() + px(20.), link.center().y);
    cx.simulate_click(after_link, Modifiers::default());
    cx.run_until_parked();
    assert_eq!(opened.borrow().len(), 1);
    cx.simulate_click(link.center(), Modifiers::default());
    cx.run_until_parked();
    assert_eq!(
        opened.borrow().last().map(String::as_str),
        Some("Some(\"file:///src/lib.rs\")")
    );
    assert!(!editor.read_with(cx, |e, _| e.is_hover_visible()));
}

#[gpui::test]
fn the_hovered_range_is_highlighted(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_hover_provider(
            cx,
            LanguageSelector::All,
            RangedHover(Some(p(0, 4)..p(0, 9))),
        );
    });
    let (editor, cx) = setup("let value = other;\n", |_| {}, cx);
    // A theme's `editor.hoverHighlightBackground`.
    let theme = Theme::from_vscode_json(
        r##"{ "type": "dark", "colors": { "editor.hoverHighlightBackground": "#ff000040" } }"##,
    )
    .unwrap();
    let color = theme.colors.hover_highlight_background;
    editor.update(cx, |e, cx| e.set_theme(Arc::new(theme), cx));
    show_at(&editor, cx, p(0, 6));
    cx.run_until_parked();
    assert_eq!(
        editor.read_with(cx, |e, cx| e.hover_highlight_for_tests(cx)),
        Some(p(0, 4)..p(0, 9))
    );
    let highlighted = editor.update(cx, |e, cx| {
        let id = cx.entity_id();
        e.model().update(cx, |m, _| {
            m.all_decorations(Some(id))
                .into_iter()
                .filter(|d| d.options.background == Some(color))
                .map(|d| (d.range.start, d.range.end))
                .collect::<Vec<_>>()
        })
    });
    assert_eq!(highlighted, vec![(4, 9)]);
    // A marker part widens it to the union.
    set_markers(
        &editor,
        cx,
        vec![marker(MarkerSeverity::Warning, p(0, 6)..p(0, 17))],
    );
    show_at(&editor, cx, p(0, 7));
    cx.run_until_parked();
    assert_eq!(
        editor.read_with(cx, |e, cx| e.hover_highlight_for_tests(cx)),
        Some(p(0, 4)..p(0, 17))
    );
    // Hiding the hover removes it.
    editor.update(cx, |e, cx| e.hide_hover(cx));
    let left = editor.update(cx, |e, cx| {
        let id = cx.entity_id();
        e.model().update(cx, |m, _| {
            m.all_decorations(Some(id))
                .into_iter()
                .filter(|d| d.options.background == Some(color))
                .count()
        })
    });
    assert_eq!(left, 0);
}

#[gpui::test]
fn hovers_without_a_range_highlight_nothing(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_hover_provider(cx, LanguageSelector::All, RangedHover(None));
    });
    let (editor, cx) = setup("let value = other;\n", |_| {}, cx);
    show_at(&editor, cx, p(0, 6));
    cx.run_until_parked();
    assert!(editor.read_with(cx, |e, _| e.is_hover_visible()));
    // Monaco's anchor is the empty range at the position.
    assert_eq!(
        editor.read_with(cx, |e, cx| e.hover_highlight_for_tests(cx)),
        None
    );
}

#[gpui::test]
fn screen_readers_read_the_hover_when_it_takes_focus(cx: &mut TestAppContext) {
    let (editor, cx) = setup("let x = 1;\n", |_| {}, cx);
    set_markers(
        &editor,
        cx,
        vec![marker(MarkerSeverity::Error, p(0, 4)..p(0, 5))],
    );
    show_at(&editor, cx, p(0, 4));
    rest(cx, 0);
    let announced = |cx: &mut VisualTestContext| {
        editor
            .read_with(cx, |e, _| e.last_announcement_for_tests())
            .map(|a| a.to_string())
    };
    // Shown, not focused: not read.
    assert_eq!(announced(cx), None);
    // Show or Focus Hover on a visible hover focuses it and reads it.
    cx.dispatch_action(gpui_monaco::actions::ShowHover::default());
    rest(cx, 0);
    assert!(cx.update(|window, cx| editor.read(cx).is_hover_focused(window, cx)));
    let text = announced(cx).expect("announced");
    // Monaco's accessible content: the parts, then the status bar.
    assert!(
        text.starts_with("Error here\n\nThis is a hover status bar.\n"),
        "{text}"
    );
    assert!(
        text.contains("It has an action with label View Problem and keybinding"),
        "{text}"
    );
}

#[gpui::test]
fn with_a_screen_reader_keyboard_hovers_take_focus_and_are_read(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        "let x = 1;\n",
        |o| {
            o.accessibility_support = gpui_monaco::options::AccessibilitySupport::On;
        },
        cx,
    );
    set_markers(
        &editor,
        cx,
        vec![marker(MarkerSeverity::Warning, p(0, 4)..p(0, 5))],
    );
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 4))], cx)
    });
    cx.dispatch_action(gpui_monaco::actions::ShowHover::default());
    rest(cx, 0);
    rest(cx, 0);
    let text = editor
        .read_with(cx, |e, _| e.last_announcement_for_tests())
        .expect("announced");
    assert!(text.starts_with("Warning here"), "{text}");
}

fn card(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> gpui::Bounds<gpui::Pixels> {
    editor
        .read_with(cx, |e, cx| e.hover_card_bounds_for_tests(cx))
        .expect("painted")
}

fn drag(
    cx: &mut VisualTestContext,
    from: gpui::Point<gpui::Pixels>,
    by: gpui::Point<gpui::Pixels>,
) {
    cx.simulate_mouse_move(from, None, Modifiers::default());
    cx.simulate_mouse_down(from, gpui::MouseButton::Left, Modifiers::default());
    paint(cx);
    let to = from + by;
    cx.simulate_mouse_move(to, gpui::MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(to, gpui::MouseButton::Left, Modifiers::default());
    paint(cx);
}

#[gpui::test]
fn resizing_from_the_full_edges(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_hover_provider(cx, LanguageSelector::All, RangedHover(None));
    });
    let (editor, cx) = setup("alpha beta\n", |_| {}, cx);
    show_at(&editor, cx, p(0, 1));
    cx.run_until_parked();
    paint(cx);
    let before = card(&editor, cx);
    // Row 0 has no room above: the card is below, with east and south
    // sashes along the whole edges.
    let east = cx.debug_bounds("hover-resize-east").expect("east sash");
    assert_eq!(east.size.height, before.size.height);
    assert!(cx.debug_bounds("hover-resize-north").is_none());
    // The east edge, near its top: only the width changes.
    drag(
        cx,
        gpui::point(east.center().x, before.top() + px(3.)),
        gpui::point(px(120.), px(30.)),
    );
    let after = card(&editor, cx);
    assert_eq!(after.size.width, before.size.width + px(120.));
    assert_eq!(after.size.height, before.size.height);
    // The south edge, near its left: only the height changes.
    let south = cx.debug_bounds("hover-resize-south").expect("south sash");
    assert_eq!(south.size.width, after.size.width);
    drag(
        cx,
        gpui::point(after.left() + px(10.), south.center().y),
        gpui::point(px(50.), px(40.)),
    );
    let last = card(&editor, cx);
    assert_eq!(last.size.width, after.size.width);
    assert_eq!(last.size.height, after.size.height + px(40.));
    assert_eq!(last.origin, before.origin);
    assert!(editor.read_with(cx, |e, _| e.is_hover_visible()));
}

/// Monaco captures the pointer on a sash: its resize cursor stays over the
/// whole window, off the sash and over the text, until the release (or a
/// move without the button, the release having been missed).
#[gpui::test]
fn a_resize_keeps_the_sash_pointer_everywhere(cx: &mut TestAppContext) {
    use gpui::CursorStyle;
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_hover_provider(cx, LanguageSelector::All, RangedHover(None));
    });
    let (editor, cx) = setup("alpha beta\n", |_| {}, cx);
    show_at(&editor, cx, p(0, 1));
    cx.run_until_parked();
    paint(cx);
    let captured =
        |cx: &mut VisualTestContext| editor.read_with(cx, |e, cx| e.captured_cursor_for_tests(cx));
    assert_eq!(captured(cx), None);
    let before = card(&editor, cx);
    let east = cx.debug_bounds("hover-resize-east").expect("east sash");
    let from = gpui::point(east.center().x, before.top() + px(3.));
    cx.simulate_mouse_move(from, None, Modifiers::default());
    cx.simulate_mouse_down(from, gpui::MouseButton::Left, Modifiers::default());
    paint(cx);
    assert_eq!(captured(cx), Some(CursorStyle::ResizeLeftRight));
    // Off the sash: over the card, the text, and outside the editor.
    for to in [
        before.center(),
        gpui::point(px(5.), px(5.)),
        from + gpui::point(px(60.), px(200.)),
    ] {
        cx.simulate_mouse_move(to, gpui::MouseButton::Left, Modifiers::default());
        paint(cx);
        assert_eq!(captured(cx), Some(CursorStyle::ResizeLeftRight));
    }
    cx.simulate_mouse_up(from, gpui::MouseButton::Left, Modifiers::default());
    paint(cx);
    assert_eq!(captured(cx), None);

    // The south sash's `ns-resize`, given back by a move without the
    // button.
    let after = card(&editor, cx);
    let south = cx.debug_bounds("hover-resize-south").expect("south sash");
    let from = gpui::point(after.left() + px(10.), south.center().y);
    cx.simulate_mouse_move(from, None, Modifiers::default());
    cx.simulate_mouse_down(from, gpui::MouseButton::Left, Modifiers::default());
    paint(cx);
    cx.simulate_mouse_move(
        gpui::point(px(5.), px(5.)),
        gpui::MouseButton::Left,
        Modifiers::default(),
    );
    paint(cx);
    assert_eq!(captured(cx), Some(CursorStyle::ResizeUpDown));
    cx.simulate_mouse_move(gpui::point(px(5.), px(5.)), None, Modifiers::default());
    paint(cx);
    assert_eq!(captured(cx), None);
    assert!(editor.read_with(cx, |e, _| e.is_hover_visible()));

    // The hover closing mid-drag takes its capture with it.
    let east = cx.debug_bounds("hover-resize-east").expect("east sash");
    let from = east.center();
    cx.simulate_mouse_move(from, None, Modifiers::default());
    cx.simulate_mouse_down(from, gpui::MouseButton::Left, Modifiers::default());
    paint(cx);
    assert_eq!(captured(cx), Some(CursorStyle::ResizeLeftRight));
    editor.update(cx, |e, cx| e.hide_hover(cx));
    paint(cx);
    assert_eq!(captured(cx), None);
}

#[gpui::test]
fn rows_and_the_status_bar_span_the_card(cx: &mut TestAppContext) {
    let (editor, cx) = setup("let x = 1;\n", |_| {}, cx);
    set_markers(
        &editor,
        cx,
        vec![marker(MarkerSeverity::Error, p(0, 4)..p(0, 5))],
    );
    show_at(&editor, cx, p(0, 4));
    cx.run_until_parked();
    paint(cx);
    let card = card(&editor, cx);
    let bar = cx.debug_bounds("hover-status-bar").expect("status bar");
    // Monaco's rows carry their own `border-top` across the card; nothing
    // insets them (the card's 1px border aside).
    assert_eq!(bar.left(), card.left() + px(1.));
    assert_eq!(bar.right(), card.right() - px(1.));
    assert_eq!(bar.bottom(), card.bottom() - px(1.));
    // Monaco's status bar: 22px rows.
    assert_eq!(bar.size.height, px(22.));
}

#[gpui::test]
fn the_marker_zone_links_the_code_too(cx: &mut TestAppContext) {
    let (editor, cx) = setup("let x = 1;\n", |_| {}, cx);
    let opened = Rc::new(RefCell::new(Vec::new()));
    let sink = opened.clone();
    cx.update(|_, cx| {
        cx.subscribe(&editor, move |_, event, _| {
            if let CodeEditorEvent::OpenLink(url) = event {
                sink.borrow_mut().push(url.to_string());
            }
        })
        .detach()
    });
    let marker = Marker {
        source: Some("lint".into()),
        code: Some("unused".into()),
        code_target: Some("https://example.com/unused".into()),
        ..marker(MarkerSeverity::Warning, p(0, 4)..p(0, 5))
    };
    set_markers(&editor, cx, vec![marker.clone()]);
    editor.update(cx, |e, cx| e.show_marker(&marker, cx));
    paint(cx);
    // Monaco `MessageWidget`'s `a.code-link`.
    let code = cx.debug_bounds("marker-code-link").expect("code link");
    cx.simulate_click(code.center(), Modifiers::default());
    cx.run_until_parked();
    assert_eq!(*opened.borrow(), ["https://example.com/unused"]);
}
