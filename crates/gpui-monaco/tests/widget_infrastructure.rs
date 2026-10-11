//! The editor's widget infrastructure (#245): the wheel, clicks, clipping
//! and focus around zones and widgets, overflowing widgets, widget inputs,
//! the registries and drags that leave the editor. Every test draws frames,
//! since hit testing, focus events and deferred widgets need one.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, AppContext as _, Bounds, Context, Entity, FocusHandle, Focusable as _,
    InteractiveElement as _, IntoElement, Modifiers, MouseButton, ParentElement as _, Pixels,
    Render, ScrollDelta, ScrollWheelEvent, Styled as _, Task, TestAppContext, VisualTestContext,
    Window, div, point, px,
};
use gpui_monaco::mouse_target::EmbeddedKind;
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, TextModel};

/// A window with the editor at (100, 200), `height` tall, and something
/// else to focus.
struct Host {
    editor: Entity<CodeEditor>,
    height: Pixels,
    other: FocusHandle,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .relative()
            .child(
                div()
                    .absolute()
                    .left(px(100.))
                    .top(px(200.))
                    .w(px(700.))
                    .h(self.height)
                    .child(self.editor.clone()),
            )
            .child(
                div()
                    .absolute()
                    .left(px(1000.))
                    .top(px(10.))
                    .size(px(20.))
                    .track_focus(&self.other),
            )
    }
}

fn setup<'a>(
    text: &str,
    height: f32,
    cx: &'a mut TestAppContext,
) -> (Entity<Host>, Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (host, cx) = cx.add_window_view(|window, cx| {
        let editor = cx.new(|cx| {
            let model = cx.new(|_| TextModel::new(&text).with_language("demo"));
            let mut editor = CodeEditor::new(model, window, cx);
            let mut options = editor.options().clone();
            options.minimap.enabled = false;
            options.quick_suggestions = gpui_monaco::options::QuickSuggestions::OFF;
            editor.update_options(options, cx);
            editor
        });
        Host {
            editor,
            height: px(height),
            other: cx.focus_handle(),
        }
    });
    let editor = host.read_with(cx, |host, _| host.editor.clone());
    // Focus events need an active window.
    cx.update(|window, _| window.activate_window());
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    draw(cx);
    (host, editor, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn rest(cx: &mut VisualTestContext, ms: u64) {
    cx.executor().advance_clock(Duration::from_millis(ms));
    cx.run_until_parked();
    draw(cx);
}

fn lines(count: usize) -> String {
    (0..count)
        .map(|i| format!("word{i} alpha beta"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn wheel(cx: &mut VisualTestContext, position: gpui::Point<Pixels>, dy: f32) {
    cx.simulate_event(ScrollWheelEvent {
        position,
        delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
        ..Default::default()
    });
    draw(cx);
}

fn scroll_top(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Pixels {
    editor.read_with(cx, |e, _| e.scroll_top())
}

/// The editor's bounds and its last widget of `kind` (the top-most).
fn widget(
    editor: &Entity<CodeEditor>,
    kind: EmbeddedKind,
    cx: &mut VisualTestContext,
) -> (Bounds<Pixels>, Bounds<Pixels>) {
    let (bounds, widgets) = editor.read_with(cx, |e, _| e.widget_bounds_for_tests());
    let widget = widgets
        .iter()
        .rev()
        .find(|(_, k)| *k == kind)
        .map(|(b, _)| *b)
        .expect("a widget of that kind");
    (bounds, widget)
}

fn set_cursor(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, row: u32, column: u32) {
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(
                row, column,
            ))],
            cx,
        )
    });
}

/// A hover of `lines` lines for every position.
struct LongHover(usize);
impl HoverProvider for LongHover {
    fn provide_hover(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<Hover>>> {
        let text: String = (0..self.0).map(|i| format!("line {i}\n\n")).collect();
        Task::ready(Ok(Some(Hover {
            contents: vec![text.into()],
            range: None,
            ..Default::default()
        })))
    }
}

/// `count` completions everywhere.
struct Many(usize);
impl CompletionProvider for Many {
    fn provide_completions(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: CompletionTrigger,
        _: &mut App,
    ) -> Task<anyhow::Result<CompletionList>> {
        Task::ready(Ok(CompletionList {
            items: (0..self.0)
                .map(|i| CompletionItem::new(format!("item{i:02}"), CompletionItemKind::Property))
                .collect(),
            incomplete: false,
        }))
    }
}

/// Ghost text completing whatever was typed with `-ghost`.
struct Ghost;
impl InlineCompletionsProvider for Ghost {
    fn provide_inline_completions(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        _: InlineCompletionContext,
        _: &mut App,
    ) -> Task<anyhow::Result<InlineCompletionList>> {
        let line = context.snapshot.line(position.row);
        Task::ready(Ok(InlineCompletionList {
            items: vec![InlineCompletion {
                insert_text: format!("{line}-ghost"),
                range: Some(
                    PointUtf16::new(position.row, 0)
                        ..PointUtf16::new(position.row, line.len() as u32),
                ),
                ..Default::default()
            }],
            suppress_suggestions: None,
            commands: Vec::new(),
        }))
    }
}

/// Signature help with a long documentation.
struct Signature;
impl SignatureHelpProvider for Signature {
    fn provide_signature_help(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: SignatureHelpTrigger,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<SignatureHelp>>> {
        Task::ready(Ok(Some(SignatureHelp {
            signatures: vec![SignatureInformation {
                label: "f(a, b)".into(),
                documentation: None,
                parameters: Vec::new(),
                active_parameter: None,
            }],
            active_signature: 0,
            active_parameter: 0,
        })))
    }
}

#[gpui::test]
fn the_wheel_over_the_hover_scrolls_it_not_the_editor(cx: &mut TestAppContext) {
    let (_, editor, cx) = setup(&lines(200), 500., cx);
    cx.update(|_, cx| register_hover_provider(cx, LanguageSelector::All, LongHover(60)));
    editor.update(cx, |e, cx| {
        e.set_scroll_top(px(400.), gpui_monaco::ScrollType::Immediate, cx)
    });
    draw(cx);
    let first = editor
        .read_with(cx, |e, cx| {
            e.visible_ranges(cx).first().map(|r| r.start.row)
        })
        .unwrap();
    set_cursor(&editor, cx, first + 5, 2);
    editor.update(cx, |e, cx| e.show_hover_at_cursor(cx));
    rest(cx, 0);
    let card = editor
        .read_with(cx, |e, cx| e.hover_card_bounds_for_tests(cx))
        .expect("hover painted");
    let top = scroll_top(&editor, cx);
    wheel(cx, card.center(), -80.);
    assert_eq!(scroll_top(&editor, cx), top, "the editor didn't scroll");
    assert!(editor.read_with(cx, |e, _| e.is_hover_visible()));
    let offset = editor
        .read_with(cx, |e, cx| e.hover_scroll_offset_for_tests(cx))
        .unwrap();
    assert!(offset.y < px(0.), "the hover scrolled: {offset:?}");
    // Off the hover, the wheel scrolls the editor (and hides the hover).
    let (bounds, _) = widget(&editor, EmbeddedKind::ContentWidget, cx);
    wheel(
        cx,
        point(bounds.left() + px(300.), bounds.bottom() - px(20.)),
        -80.,
    );
    assert!(scroll_top(&editor, cx) > top);
}

#[gpui::test]
fn the_wheel_over_the_suggest_list_scrolls_it(cx: &mut TestAppContext) {
    let (_, editor, cx) = setup(&lines(100), 500., cx);
    cx.update(|_, cx| register_completion_provider(cx, LanguageSelector::All, Many(40)));
    editor.update(cx, |e, cx| {
        e.set_scroll_top(px(200.), gpui_monaco::ScrollType::Immediate, cx)
    });
    draw(cx);
    let first = editor
        .read_with(cx, |e, cx| {
            e.visible_ranges(cx).first().map(|r| r.start.row)
        })
        .unwrap();
    set_cursor(&editor, cx, first + 3, 0);
    cx.simulate_keystrokes("ctrl-space");
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_suggest_visible()));
    let (_, list) = widget(&editor, EmbeddedKind::ContentWidget, cx);
    let top = scroll_top(&editor, cx);
    wheel(cx, list.center(), -100.);
    assert_eq!(scroll_top(&editor, cx), top, "the editor didn't scroll");
    let (first_row, shown) = editor
        .read_with(cx, |e, cx| e.suggest_rows_shown_for_tests(cx))
        .unwrap();
    assert!(first_row > 0, "the list scrolled");
    assert!(shown < 40);
    assert!(editor.read_with(cx, |e, _| e.is_suggest_visible()));
}

#[gpui::test]
fn the_wheel_over_menus_and_peek_leaves_the_editor_alone(cx: &mut TestAppContext) {
    let (_, editor, cx) = setup(&lines(200), 500., cx);
    editor.update(cx, |e, cx| {
        e.set_scroll_top(px(300.), gpui_monaco::ScrollType::Immediate, cx)
    });
    draw(cx);
    let top = scroll_top(&editor, cx);
    // The context menu.
    let (bounds, _) = editor.read_with(cx, |e, _| e.widget_bounds_for_tests());
    let at = bounds.origin + point(px(200.), px(100.));
    cx.update(|window, cx| editor.update(cx, |e, cx| e.show_context_menu(Some(at), window, cx)));
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_context_menu_visible()));
    let (_, menu) = widget(&editor, EmbeddedKind::OverlayWidget, cx);
    wheel(cx, menu.center(), -60.);
    assert_eq!(scroll_top(&editor, cx), top);
    assert!(editor.read_with(cx, |e, _| e.is_context_menu_visible()));
    editor.update(cx, |e, cx| e.hide_context_menu(cx));
    draw(cx);

    // A peek view's results.
    let first = editor
        .read_with(cx, |e, cx| {
            e.visible_ranges(cx).first().map(|r| r.start.row)
        })
        .unwrap();
    let locations = (0..60)
        .map(|row| Location {
            uri: None,
            range: PointUtf16::new(row, 0)..PointUtf16::new(row, 4),
            ..Default::default()
        })
        .collect();
    editor.update(cx, |e, cx| {
        e.peek_locations(PointUtf16::new(first + 2, 0), locations, None, cx)
    });
    draw(cx);
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_peek_visible()));
    let top = scroll_top(&editor, cx);
    let (_, zone) = widget(&editor, EmbeddedKind::ViewZone, cx);
    // The results list is on the right.
    let results = point(zone.right() - px(60.), zone.center().y);
    wheel(cx, results, -60.);
    assert_eq!(scroll_top(&editor, cx), top, "the editor didn't scroll");
    assert!(editor.read_with(cx, |e, _| e.is_peek_visible()));
}

#[gpui::test]
fn right_and_middle_clicks_in_widgets_keep_them_open(cx: &mut TestAppContext) {
    let (_, editor, cx) = setup(&lines(50), 500., cx);
    // The quick input: a right-click inside doesn't move focus to the
    // editor, which would close it.
    cx.update(|window, cx| editor.update(cx, |e, cx| e.open_quick_access(">", window, cx)));
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_quick_access_open()));
    let (_, quick) = widget(&editor, EmbeddedKind::OverlayWidget, cx);
    let inside = point(quick.center().x, quick.bottom() - px(10.));
    cx.simulate_mouse_down(inside, MouseButton::Right, Modifiers::default());
    cx.simulate_mouse_up(inside, MouseButton::Right, Modifiers::default());
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_quick_access_open()));
    assert!(!cx.update(|window, cx| editor.read(cx).focus_handle(cx).is_focused(window)));
    // Middle-click neither: no autoscroll, no focus.
    cx.simulate_mouse_down(inside, MouseButton::Middle, Modifiers::default());
    cx.simulate_mouse_up(inside, MouseButton::Middle, Modifiers::default());
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_quick_access_open()));
    cx.simulate_keystrokes("escape");
    draw(cx);

    // The context menu.
    let (bounds, _) = editor.read_with(cx, |e, _| e.widget_bounds_for_tests());
    let at = bounds.origin + point(px(200.), px(100.));
    cx.update(|window, cx| editor.update(cx, |e, cx| e.show_context_menu(Some(at), window, cx)));
    draw(cx);
    let (_, menu) = widget(&editor, EmbeddedKind::OverlayWidget, cx);
    cx.simulate_mouse_down(menu.center(), MouseButton::Right, Modifiers::default());
    cx.simulate_mouse_up(menu.center(), MouseButton::Right, Modifiers::default());
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_context_menu_visible()));
}

#[gpui::test]
fn focusing_the_hover_keeps_it_open(cx: &mut TestAppContext) {
    let (_, editor, cx) = setup(&lines(20), 500., cx);
    cx.update(|_, cx| register_hover_provider(cx, LanguageSelector::All, LongHover(60)));
    set_cursor(&editor, cx, 8, 2);
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    let chord = format!("{modifier}-k {modifier}-i");
    // Cmd/Ctrl+K Cmd/Ctrl+I shows the hover; again, it focuses it.
    cx.simulate_keystrokes(&chord);
    rest(cx, 0);
    assert!(editor.read_with(cx, |e, _| e.is_hover_visible()));
    cx.simulate_keystrokes(&chord);
    rest(cx, 0);
    draw(cx);
    assert!(
        editor.read_with(cx, |e, _| e.is_hover_visible()),
        "focusing the hover doesn't close it"
    );
    assert!(cx.update(|window, cx| editor.read(cx).is_hover_focused(window, cx)));
    // The keyboard scrolls it, in a drawn frame.
    let offset = |cx: &mut VisualTestContext| {
        editor
            .read_with(cx, |e, cx| e.hover_scroll_offset_for_tests(cx))
            .unwrap()
            .y
    };
    cx.simulate_keystrokes("pagedown");
    draw(cx);
    assert!(offset(cx) < px(0.));
    // So does the wheel.
    let before = offset(cx);
    let card = editor
        .read_with(cx, |e, cx| e.hover_card_bounds_for_tests(cx))
        .unwrap();
    wheel(cx, card.center(), -40.);
    assert!(offset(cx) < before);
    assert!(editor.read_with(cx, |e, _| e.is_hover_visible()));
    // Escape closes it and returns to the text.
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_hover_visible()));
    assert!(cx.update(|window, cx| editor.read(cx).focus_handle(cx).is_focused(window)));
}

#[gpui::test]
fn suggest_and_parameter_hints_close_when_the_editor_loses_focus(cx: &mut TestAppContext) {
    let (host, editor, cx) = setup(&lines(20), 500., cx);
    cx.update(|_, cx| {
        register_completion_provider(cx, LanguageSelector::All, Many(5));
        register_signature_help_provider(cx, LanguageSelector::All, Signature);
    });
    set_cursor(&editor, cx, 3, 0);
    cx.simulate_keystrokes("ctrl-space");
    draw(cx);
    editor.update(cx, |e, cx| {
        e.trigger_signature_help(SignatureHelpTrigger::Invoke, cx)
    });
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_suggest_visible()));
    assert!(editor.read_with(cx, |e, _| e.is_signature_help_visible()));
    // Focus within the editor's widgets keeps them: the hover.
    cx.update(|_, cx| register_hover_provider(cx, LanguageSelector::All, LongHover(3)));
    cx.dispatch_action(gpui_monaco::actions::ShowHover);
    rest(cx, 0);
    cx.dispatch_action(gpui_monaco::actions::ShowHover);
    rest(cx, 0);
    assert!(cx.update(|window, cx| editor.read(cx).is_hover_focused(window, cx)));
    assert!(editor.read_with(cx, |e, _| e.is_suggest_visible()));
    assert!(editor.read_with(cx, |e, _| e.is_signature_help_visible()));
    // Focus elsewhere in the window closes them (Monaco
    // `onDidBlurEditorWidget`).
    let other = host.read_with(cx, |host, _| host.other.clone());
    cx.update(|window, cx| window.focus(&other, cx));
    draw(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_suggest_visible()));
    assert!(!editor.read_with(cx, |e, _| e.is_signature_help_visible()));
}

#[gpui::test]
fn overflowing_widgets_extend_past_a_short_editor(cx: &mut TestAppContext) {
    // Three lines tall.
    let (_, editor, cx) = setup(&lines(3), 60., cx);
    cx.update(|_, cx| {
        register_completion_provider(cx, LanguageSelector::All, Many(30));
        register_hover_provider(cx, LanguageSelector::All, LongHover(10));
    });
    set_cursor(&editor, cx, 1, 0);
    cx.simulate_keystrokes("ctrl-space");
    draw(cx);
    let (bounds, list) = widget(&editor, EmbeddedKind::ContentWidget, cx);
    assert!(list.size.height > bounds.size.height * 2., "{list:?}");
    assert!(list.bottom() > bounds.bottom() || list.top() < bounds.top());
    // Twelve rows show, not just what fits in the editor.
    let (_, shown) = editor
        .read_with(cx, |e, cx| e.suggest_rows_shown_for_tests(cx))
        .unwrap();
    assert_eq!(shown, 12);
    cx.simulate_keystrokes("escape");
    draw(cx);

    // The hover: in full, outside the editor.
    editor.update(cx, |e, cx| e.show_hover_at_cursor(cx));
    rest(cx, 0);
    let card = editor
        .read_with(cx, |e, cx| e.hover_card_bounds_for_tests(cx))
        .unwrap();
    assert!(card.size.height > bounds.size.height, "{card:?}");

    // The context menu, from the editor's top: it fits in the window.
    let at = bounds.origin + point(px(20.), px(20.));
    cx.update(|window, cx| editor.update(cx, |e, cx| e.show_context_menu(Some(at), window, cx)));
    draw(cx);
    let (_, menu) = widget(&editor, EmbeddedKind::OverlayWidget, cx);
    assert!(menu.size.height > bounds.size.height);
}

#[gpui::test]
fn overflowing_widgets_flip_and_shrink_near_the_window_edge(cx: &mut TestAppContext) {
    // An editor near the window's bottom: suggest goes above the line.
    cx.update(gpui_monaco::init);
    let (host, cx) = cx.add_window_view(|window, cx| {
        let editor = cx.new(|cx| {
            let model = cx.new(|_| TextModel::new(&lines(200)));
            CodeEditor::new(model, window, cx)
        });
        Host {
            editor,
            height: px(860.),
            other: cx.focus_handle(),
        }
    });
    let editor = host.read_with(cx, |host, _| host.editor.clone());
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.update(|_, cx| register_completion_provider(cx, LanguageSelector::All, Many(30)));
    draw(cx);
    // The last line that shows fully, near the window's bottom.
    let last = editor
        .read_with(cx, |e, cx| e.visible_ranges(cx).last().map(|r| r.end.row))
        .unwrap()
        - 2;
    set_cursor(&editor, cx, last, 0);
    cx.simulate_keystrokes("ctrl-space");
    draw(cx);
    let line = editor
        .read_with(cx, |e, _| {
            e.scrolled_visible_position(PointUtf16::new(last, 0))
        })
        .unwrap();
    let (_, list) = widget(&editor, EmbeddedKind::ContentWidget, cx);
    assert!(
        list.bottom() <= line.y + px(1.),
        "above the line: {list:?} {line:?}"
    );
    let window = cx.update(|window, _| window.viewport_size());
    assert!(list.bottom() <= window.height);
}

#[gpui::test]
fn widget_inputs_are_plain_inputs(cx: &mut TestAppContext) {
    let (_, editor, cx) = setup(&lines(10), 500., cx);
    cx.update(|_, cx| {
        register_completion_provider(cx, LanguageSelector::All, Many(5));
        register_inline_completions_provider(cx, LanguageSelector::All, Ghost);
        register_hover_provider(cx, LanguageSelector::All, LongHover(2));
        gpui_monaco::set_editor_zoom_level(2., cx);
    });
    cx.update(|window, cx| editor.update(cx, |e, cx| e.open_quick_access("", window, cx)));
    draw(cx);
    let input = editor
        .read_with(cx, |e, cx| e.quick_access_input_for_tests(cx))
        .expect("quick input");
    // Typing gives no ghost text or suggestions, and Ctrl+Space no list.
    cx.simulate_input("fo");
    rest(cx, 200);
    cx.simulate_keystrokes("ctrl-space");
    rest(cx, 200);
    input.read_with(cx, |input, cx| {
        assert_eq!(input.model().read(cx).text(), "fo");
        assert!(!input.is_suggest_visible());
        assert!(!input.inline_suggestion_visible(cx));
        assert!(input.ghost_text_for_tests(cx).is_none());
        // The input background and the UI font at 13px, unzoomed.
        let colors = input.colors();
        assert_eq!(colors.background, colors.input_background);
        let options = input.options();
        assert_eq!(options.font_size, px(13.));
        assert_eq!(
            options.hover.enabled,
            gpui_monaco::options::HoverEnabled::Off
        );
        assert!(!options.inline_suggest.enabled);
    });
    // The editor itself still has them.
    editor.read_with(cx, |e, _| {
        assert!(e.options().inline_suggest.enabled);
    });
}

#[gpui::test]
fn registries_leave_out_internal_editors_and_models(cx: &mut TestAppContext) {
    let (_, editor, cx) = setup(&lines(10), 500., cx);
    let created = Rc::new(Cell::new(0));
    let models_created = Rc::new(Cell::new(0));
    let _subscriptions = cx.update(|_, cx| {
        let editors = created.clone();
        let models = models_created.clone();
        (
            gpui_monaco::on_did_create_editor(cx, move |_, _| editors.set(editors.get() + 1)),
            gpui_monaco::on_did_create_model(cx, move |_, _| models.set(models.get() + 1)),
        )
    });
    let count = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            (
                gpui_monaco::editors(cx).len(),
                gpui_monaco::models(cx).len(),
            )
        })
    };
    assert_eq!(count(cx), (1, 1));
    // The find widget (find and replace inputs), quick access and a peek
    // view's placeholder model.
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    cx.simulate_keystrokes(&format!("{modifier}-f"));
    draw(cx);
    cx.update(|window, cx| editor.update(cx, |e, cx| e.open_quick_access(">", window, cx)));
    draw(cx);
    editor.update(cx, |e, cx| {
        let locations = (0..3)
            .map(|row| Location {
                uri: None,
                range: PointUtf16::new(row, 0)..PointUtf16::new(row, 2),
                ..Default::default()
            })
            .collect();
        e.peek_locations(PointUtf16::new(5, 0), locations, None, cx)
    });
    draw(cx);
    // The peek view's preview is a real (embedded) editor, as in Monaco;
    // the inputs and the placeholder model aren't listed.
    let (editors, models) = count(cx);
    assert_eq!(models, 1);
    assert_eq!(editors, 2, "the main editor and the peek preview");
    assert_eq!(created.get(), 1);
    assert_eq!(models_created.get(), 0);
}

#[gpui::test]
fn drags_follow_the_mouse_out_of_the_editor(cx: &mut TestAppContext) {
    let (_, editor, cx) = setup(&lines(200), 400., cx);
    editor.update(cx, |e, cx| {
        e.set_scroll_top(px(1000.), gpui_monaco::ScrollType::Immediate, cx)
    });
    draw(cx);
    let first = editor
        .read_with(cx, |e, cx| {
            e.visible_ranges(cx).first().map(|r| r.start.row)
        })
        .unwrap();
    // Drag-select from a line up out of the editor (above it, over the
    // window), and hold the mouse there: it scrolls up on a timer, and the
    // selection follows.
    let start = editor
        .read_with(cx, |e, _| {
            e.scrolled_visible_position(PointUtf16::new(first + 5, 2))
        })
        .unwrap()
        + point(px(1.), px(2.));
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    draw(cx);
    let outside = point(start.x, px(120.));
    cx.simulate_mouse_move(outside, Some(MouseButton::Left), Modifiers::default());
    draw(cx);
    let top = scroll_top(&editor, cx);
    rest(cx, 200);
    rest(cx, 200);
    assert!(scroll_top(&editor, cx) < top, "it scrolled up");
    let selection = editor.read_with(cx, |e, _| e.selections()[0]);
    assert!(selection.head.row < first, "{selection:?}");
    assert_eq!(selection.anchor, PointUtf16::new(first + 5, 2));
    // Back inside, it stops.
    let inside = start + point(px(0.), px(40.));
    cx.simulate_mouse_move(inside, Some(MouseButton::Left), Modifiers::default());
    draw(cx);
    let top = scroll_top(&editor, cx);
    rest(cx, 200);
    assert_eq!(scroll_top(&editor, cx), top);
    cx.simulate_mouse_up(inside, MouseButton::Left, Modifiers::default());
    draw(cx);

    // The scrollbar thumb, dragged below the editor.
    let (_, thumb) = editor
        .read_with(cx, |e, _| e.vertical_scrollbar_for_tests())
        .expect("a scrollbar");
    let grab = thumb.center();
    cx.simulate_mouse_down(grab, MouseButton::Left, Modifiers::default());
    draw(cx);
    let before = scroll_top(&editor, cx);
    cx.simulate_mouse_move(
        point(grab.x - px(300.), px(1000.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    draw(cx);
    assert!(scroll_top(&editor, cx) > before, "the thumb followed");
    cx.simulate_mouse_up(
        point(grab.x, px(1000.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    draw(cx);
}

#[gpui::test]
fn zones_scrolled_out_take_no_clicks_outside_the_text(cx: &mut TestAppContext) {
    let (_, editor, cx) = setup(&lines(100), 300., cx);
    let clicks = Rc::new(RefCell::new(0));
    struct Zone(Rc<RefCell<usize>>);
    impl Render for Zone {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let clicks = self.0.clone();
            div()
                .size_full()
                .on_mouse_down(MouseButton::Left, move |_, _, _| *clicks.borrow_mut() += 1)
        }
    }
    let zone = cx.update(|_, cx| cx.new(|_| Zone(clicks.clone())));
    editor.update(cx, |e, cx| {
        e.add_view_zone(
            gpui_monaco::ViewZone {
                after_line: 10,
                height: gpui_monaco::ZoneHeight::Pixels(px(200.)),
                view: zone.into(),
            },
            cx,
        );
    });
    draw(cx);
    // Scroll so the zone is half above the editor.
    let (bounds, _) = editor.read_with(cx, |e, _| e.widget_bounds_for_tests());
    let (zone_top, _) = {
        let (_, zone) = widget(&editor, EmbeddedKind::ViewZone, cx);
        (zone.top(), zone)
    };
    editor.update(cx, |e, cx| {
        let top = e.scroll_top() + (zone_top - bounds.top()) + px(100.);
        e.set_scroll_top(top, gpui_monaco::ScrollType::Immediate, cx)
    });
    draw(cx);
    let (_, visible) = widget(&editor, EmbeddedKind::ViewZone, cx);
    assert_eq!(visible.top(), bounds.top(), "clipped to the editor");
    // A click above the editor, where the hidden half is, doesn't reach it.
    let above = point(visible.center().x, bounds.top() - px(50.));
    cx.simulate_click(above, Modifiers::default());
    draw(cx);
    assert_eq!(*clicks.borrow(), 0);
    // Inside, it does.
    cx.simulate_click(visible.center(), Modifiers::default());
    draw(cx);
    assert_eq!(*clicks.borrow(), 1);
}
