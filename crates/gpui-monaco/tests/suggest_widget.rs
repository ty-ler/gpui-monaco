//! The suggest widget's and parameter hints' layout and mouse handling
//! (#248): the list's scrollbar, sizing to the room around the cursor
//! without flipping while typing, the north resize handle, the details
//! panel's placement west of the list and its resizing,
//! `suggest.fit_width_to_details`, the clickable status bar, and parameter
//! hints' maximum height; and (#292) the list's pixel scrolling, the
//! scrollbars' fading, the details panel's height in the frame its content
//! changes, and clicks through the empty corners. Every test draws frames,
//! since the widgets lay themselves out against the window.

use std::time::Duration;

use gpui::{
    App, AppContext as _, Bounds, Context, Entity, Focusable as _, IntoElement, Modifiers,
    MouseButton, ParentElement as _, Pixels, Point, Render, ScrollDelta, ScrollWheelEvent,
    Styled as _, Task, TestAppContext, VisualTestContext, Window, div, point, px,
};
use gpui_monaco::ContentWidgetPreference;
use gpui_monaco::mouse_target::EmbeddedKind;
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, EditorOptions, TextModel};

/// A window with the editor at `origin`, `size` big.
struct Host {
    editor: Entity<CodeEditor>,
    origin: Point<Pixels>,
    size: gpui::Size<Pixels>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().relative().child(
            div()
                .absolute()
                .left(self.origin.x)
                .top(self.origin.y)
                .w(self.size.width)
                .h(self.size.height)
                .child(self.editor.clone()),
        )
    }
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

/// An editor at `origin` of `size` (`None`: 100, 200 and 700 × 500) in a
/// window, with `text`, focused.
fn setup<'a>(
    text: &str,
    place: Option<(Point<Pixels>, gpui::Size<Pixels>)>,
    options: impl FnOnce(&mut EditorOptions),
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (origin, size) =
        place.unwrap_or((point(px(100.), px(200.)), gpui::size(px(700.), px(500.))));
    let (host, cx) = cx.add_window_view(|window, cx| {
        let editor = cx.new(|cx| {
            let model = cx.new(|_| TextModel::new(&text).with_language("demo"));
            let mut editor = CodeEditor::new(model, window, cx);
            let mut opts = editor.options().clone();
            opts.minimap.enabled = false;
            opts.quick_suggestions = gpui_monaco::options::QuickSuggestions::OFF;
            options(&mut opts);
            editor.update_options(opts, cx);
            editor
        });
        Host {
            editor,
            origin,
            size,
        }
    });
    let editor = host.read_with(cx, |host, _| host.editor.clone());
    cx.update(|window, _| window.activate_window());
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    draw(cx);
    (editor, cx)
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

fn wheel(cx: &mut VisualTestContext, position: Point<Pixels>, dy: f32) {
    cx.simulate_event(ScrollWheelEvent {
        position,
        delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
        ..Default::default()
    });
    draw(cx);
}

/// Presses at `from`, drags to `to` and releases.
fn drag(cx: &mut VisualTestContext, from: Point<Pixels>, to: Point<Pixels>) {
    let none = Modifiers::none();
    cx.simulate_mouse_move(from, None, none);
    cx.simulate_mouse_down(from, MouseButton::Left, none);
    let middle = point((from.x + to.x) / 2., (from.y + to.y) / 2.);
    cx.simulate_mouse_move(middle, Some(MouseButton::Left), none);
    cx.simulate_mouse_move(to, Some(MouseButton::Left), none);
    cx.simulate_mouse_up(to, MouseButton::Left, none);
    draw(cx);
}

/// The list's rows and the details panel's content, as painted.
fn painted(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
) -> (Bounds<Pixels>, Option<Bounds<Pixels>>) {
    editor
        .read_with(cx, |e, cx| e.suggest_painted_bounds_for_tests(cx))
        .expect("the list painted")
}

fn first_row(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> usize {
    editor
        .read_with(cx, |e, cx| e.suggest_rows_shown_for_tests(cx))
        .unwrap()
        .0
}

fn line_top(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, row: u32) -> Pixels {
    editor
        .read_with(cx, |e, _| {
            e.scrolled_visible_position(PointUtf16::new(row, 0))
        })
        .unwrap()
        .y
}

/// Items whose labels start with the typed word: `count` of `item00`…
/// and one `other`, with details and docs.
struct Items(usize);
impl CompletionProvider for Items {
    fn provide_completions(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: CompletionTrigger,
        _: &mut App,
    ) -> Task<anyhow::Result<CompletionList>> {
        let mut items: Vec<CompletionItem> = (0..self.0)
            .map(|i| CompletionItem {
                detail: Some(format!("fn item{i:02}(&self) -> u32").into()),
                documentation: Some(
                    format!("Docs for item {i}.\n\nMore about it.")
                        .to_string()
                        .into(),
                ),
                ..CompletionItem::new(format!("item{i:02}"), CompletionItemKind::Method)
            })
            .collect();
        items.push(CompletionItem::new("other", CompletionItemKind::Property));
        Task::ready(Ok(CompletionList {
            items,
            incomplete: false,
        }))
    }
}

fn provide(cx: &mut VisualTestContext, count: usize) {
    cx.update(|_, cx| register_completion_provider(cx, LanguageSelector::All, Items(count)));
}

/// Monaco's list consumes the wheel and has a scrollbar
/// (`ScrollbarVisibility.Auto`): it shows after scrolling, and its track
/// and slider scroll the list.
#[gpui::test]
fn the_list_has_a_scrollbar(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(30), None, |_| {}, cx);
    provide(cx, 40);
    set_cursor(&editor, cx, 2, 0);
    cx.simulate_keystrokes("ctrl-space");
    draw(cx);
    assert!(!editor.read_with(cx, |e, cx| e.suggest_scrollbars_revealed_for_tests(cx)));
    let (rows, _) = painted(&editor, cx);
    // The wheel scrolls the list and shows the scrollbar for a while.
    wheel(cx, rows.center(), -60.);
    assert!(first_row(&editor, cx) > 0);
    assert!(editor.read_with(cx, |e, cx| e.suggest_scrollbars_revealed_for_tests(cx)));
    rest(cx, 600);
    assert!(!editor.read_with(cx, |e, cx| e.suggest_scrollbars_revealed_for_tests(cx)));
    // With the mouse over the list, a click near the track's bottom
    // scrolls to the end (Monaco centers the slider there).
    let track_x = rows.right() - px(5.);
    cx.simulate_mouse_move(rows.center(), None, Modifiers::none());
    draw(cx);
    cx.simulate_click(point(track_x, rows.bottom() - px(2.)), Modifiers::none());
    draw(cx);
    // 41 rows, 12 showing.
    assert_eq!(first_row(&editor, cx), 29);
    // Dragging the slider (now at the bottom) to the top scrolls back.
    drag(
        cx,
        point(track_x, rows.bottom() - px(5.)),
        point(track_x, rows.top() - px(100.)),
    );
    assert_eq!(first_row(&editor, cx), 0);
    assert!(editor.read_with(cx, |e, _| e.is_suggest_visible()));
}

/// An editor whose last lines are near the window's bottom, so the list
/// goes above them.
fn near_the_bottom(
    options: impl FnOnce(&mut EditorOptions),
    cx: &mut TestAppContext,
) -> (Entity<CodeEditor>, &mut VisualTestContext, u32) {
    let window = cx.update(|cx| Bounds::maximized(None, cx).size);
    let place = (
        point(px(100.), window.height - px(400.)),
        gpui::size(px(700.), px(380.)),
    );
    let (editor, cx) = setup(&lines(10), Some(place), options, cx);
    // A line about 100px from the window's bottom.
    let row = (0..10)
        .rev()
        .find(|row| line_top(&editor, cx, *row) < window.height - px(110.))
        .unwrap();
    (editor, cx, row)
}

/// Monaco `SuggestWidget._layout`: the list goes above when there's no room
/// below, and doesn't flip back while typing narrows it to a row that
/// would fit below.
#[gpui::test]
fn the_list_keeps_its_side_while_typing(cx: &mut TestAppContext) {
    let (editor, cx, row) = near_the_bottom(|_| {}, cx);
    provide(cx, 40);
    set_cursor(&editor, cx, row, 0);
    cx.simulate_input("i");
    cx.simulate_keystrokes("ctrl-space");
    draw(cx);
    let (side, size) = editor
        .read_with(cx, |e, cx| e.suggest_list_layout_for_tests(cx))
        .unwrap();
    assert_eq!(side, ContentWidgetPreference::Above);
    assert!(size.height > px(100.), "{size:?}");
    let (rows, _) = painted(&editor, cx);
    assert!(rows.bottom() <= line_top(&editor, cx, row));
    // One row left: it would fit below, but stays above.
    cx.simulate_input("tem39");
    draw(cx);
    assert_eq!(
        editor.read_with(cx, |e, _| e.suggest_labels_for_tests()),
        Some(vec!["item39".to_string()])
    );
    let (side, _) = editor
        .read_with(cx, |e, cx| e.suggest_list_layout_for_tests(cx))
        .unwrap();
    assert_eq!(side, ContentWidgetPreference::Above);
    let (rows, _) = painted(&editor, cx);
    assert!(rows.bottom() <= line_top(&editor, cx, row));
    // Deleting brings the rows back at the wanted height, still above.
    for _ in 0..5 {
        cx.simulate_keystrokes("backspace");
    }
    draw(cx);
    let (side, grown) = editor
        .read_with(cx, |e, cx| e.suggest_list_layout_for_tests(cx))
        .unwrap();
    assert_eq!(side, ContentWidgetPreference::Above);
    assert_eq!(grown, size);
}

/// Monaco enables the north sash while the list is above the cursor.
#[gpui::test]
fn the_north_sash_resizes_a_list_above(cx: &mut TestAppContext) {
    let (editor, cx, row) = near_the_bottom(|_| {}, cx);
    provide(cx, 40);
    set_cursor(&editor, cx, row, 0);
    cx.simulate_keystrokes("ctrl-space");
    draw(cx);
    let (_, before) = editor
        .read_with(cx, |e, cx| e.suggest_list_layout_for_tests(cx))
        .unwrap();
    let (rows, _) = painted(&editor, cx);
    // The border's top edge, dragged up by 3 rows of 19px or so.
    let edge = point(rows.left() + px(100.), rows.top() - px(1.));
    drag(cx, edge, edge - point(px(0.), px(60.)));
    let (side, after) = editor
        .read_with(cx, |e, cx| e.suggest_list_layout_for_tests(cx))
        .unwrap();
    assert_eq!(side, ContentWidgetPreference::Above);
    assert!(
        after.height > before.height + px(40.),
        "{before:?} {after:?}"
    );
    let stored = cx
        .update(|_, cx| CodeEditor::suggest_size_for_tests(cx))
        .unwrap();
    assert!(stored.height > before.height + px(40.));
    // The bottom stays at the line.
    let (rows, _) = painted(&editor, cx);
    assert!(rows.bottom() <= line_top(&editor, cx, row));
}

/// Monaco captures the pointer on a sash: its resize cursor stays over the
/// whole window, over the text too, until the release (GPUI's drags keep
/// the dragged element's cursor).
#[gpui::test]
fn dragging_a_sash_keeps_its_pointer_everywhere(cx: &mut TestAppContext) {
    let (editor, cx, row) = near_the_bottom(|_| {}, cx);
    provide(cx, 40);
    set_cursor(&editor, cx, row, 0);
    cx.simulate_keystrokes("ctrl-space");
    draw(cx);
    let captured =
        |cx: &mut VisualTestContext| editor.read_with(cx, |e, cx| e.captured_cursor_for_tests(cx));
    assert_eq!(captured(cx), None);
    let (rows, _) = painted(&editor, cx);
    let edge = point(rows.left() + px(100.), rows.top() - px(1.));
    let none = Modifiers::none();
    cx.simulate_mouse_move(edge, None, none);
    cx.simulate_mouse_down(edge, MouseButton::Left, none);
    for to in [edge - point(px(0.), px(20.)), point(px(5.), px(5.))] {
        cx.simulate_mouse_move(to, Some(MouseButton::Left), none);
        draw(cx);
        assert_eq!(captured(cx), Some(gpui::CursorStyle::ResizeRow));
    }
    cx.simulate_mouse_up(point(px(5.), px(5.)), MouseButton::Left, none);
    draw(cx);
    assert_eq!(captured(cx), None);
}

/// Monaco `SuggestDetailsOverlay._placeAtAnchor`: without room on the
/// right, the details go west of the list.
#[gpui::test]
fn the_details_go_west_near_the_right_edge(cx: &mut TestAppContext) {
    let window = cx.update(|cx| Bounds::maximized(None, cx).size);
    let place = (
        point(window.width - px(560.), px(200.)),
        gpui::size(px(540.), px(500.)),
    );
    let (editor, cx) = setup(&lines(30), Some(place), |_| {}, cx);
    provide(cx, 5);
    set_cursor(&editor, cx, 2, 0);
    cx.simulate_keystrokes("ctrl-space");
    draw(cx);
    // Ctrl+Space again shows the details.
    cx.simulate_keystrokes("ctrl-space");
    draw(cx);
    let (side, bounds) = editor
        .read_with(cx, |e, cx| e.suggest_details_placement_for_tests(cx))
        .expect("details shown");
    assert_eq!(side, "west");
    // Its right edge meets the list's left.
    assert_eq!(bounds.right(), px(-1.));
    let (rows, details) = painted(&editor, cx);
    let details = details.unwrap();
    assert!(details.right() <= rows.left());
    // The content widget takes the mouse over both.
    let (_, widgets) = editor.read_with(cx, |e, _| e.widget_bounds_for_tests());
    let over = |at: Point<Pixels>| {
        widgets
            .iter()
            .any(|(bounds, kind)| *kind == EmbeddedKind::ContentWidget && bounds.contains(&at))
    };
    assert!(over(details.center()) && over(rows.center()));
}

/// The details panel is sized to its content and resizable; the size is
/// kept while the session lasts (Monaco `_userSize`).
#[gpui::test]
fn the_details_are_resizable(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(30), None, |_| {}, cx);
    provide(cx, 40);
    set_cursor(&editor, cx, 2, 0);
    cx.simulate_keystrokes("ctrl-space");
    draw(cx);
    cx.simulate_keystrokes("ctrl-space");
    draw(cx);
    let (side, before) = editor
        .read_with(cx, |e, cx| e.suggest_details_placement_for_tests(cx))
        .unwrap();
    assert_eq!(side, "east");
    assert_eq!(before.size.width, px(330.));
    // Sized to its two paragraphs and the type, not the list's height.
    let (rows, _) = painted(&editor, cx);
    assert!(before.size.height < rows.size.height, "{before:?}");
    // Drag the east edge 60px right.
    let edge = point(
        rows.left() - px(1.) + before.right() - px(1.),
        rows.top() + px(20.),
    );
    drag(cx, edge, edge + point(px(60.), px(0.)));
    let (_, after) = editor
        .read_with(cx, |e, cx| e.suggest_details_placement_for_tests(cx))
        .unwrap();
    assert!((after.size.width - px(390.)).abs() < px(2.), "{after:?}");
    // Kept when another item is focused.
    cx.simulate_keystrokes("down");
    draw(cx);
    let (_, moved) = editor
        .read_with(cx, |e, cx| e.suggest_details_placement_for_tests(cx))
        .unwrap();
    assert_eq!(moved.size.width, after.size.width);
    // A new session starts at the default width again.
    cx.simulate_keystrokes("escape");
    cx.simulate_keystrokes("ctrl-space");
    draw(cx);
    let (_, again) = editor
        .read_with(cx, |e, cx| e.suggest_details_placement_for_tests(cx))
        .unwrap();
    assert_eq!(again.size.width, px(330.));
}

/// Long labels with inline details.
struct Long;
impl CompletionProvider for Long {
    fn provide_completions(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: CompletionTrigger,
        _: &mut App,
    ) -> Task<anyhow::Result<CompletionList>> {
        Task::ready(Ok(CompletionList {
            items: vec![CompletionItem {
                label_detail: Some("(first: u32, second: u32, third: u32)".into()),
                label_description: Some("some::rather::long::module::path".into()),
                ..CompletionItem::new("a_function_with_a_long_name", CompletionItemKind::Function)
            }],
            incomplete: false,
        }))
    }
}

/// `suggest.fitWidthToDetails` widens the list to its widest label and
/// details, until the list is resized.
#[gpui::test]
fn fit_width_to_details_widens_the_list(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        &lines(30),
        None,
        |o| o.suggest.fit_width_to_details = true,
        cx,
    );
    cx.update(|_, cx| register_completion_provider(cx, LanguageSelector::All, Long));
    set_cursor(&editor, cx, 2, 0);
    cx.simulate_keystrokes("ctrl-space");
    draw(cx);
    let (_, fitted) = editor
        .read_with(cx, |e, cx| e.suggest_list_layout_for_tests(cx))
        .unwrap();
    assert!(fitted.width > px(430.), "{fitted:?}");
    // Capped to the editor's width.
    assert!(fitted.width <= px(700.));
    // Off, or once resized: the default width.
    cx.simulate_keystrokes("escape");
    cx.update(|_, cx| CodeEditor::set_suggest_size_for_tests(gpui::size(px(430.), px(200.)), cx));
    cx.simulate_keystrokes("ctrl-space");
    draw(cx);
    let (_, resized) = editor
        .read_with(cx, |e, cx| e.suggest_list_layout_for_tests(cx))
        .unwrap();
    assert_eq!(resized.width, px(430.));
}

/// Monaco's status bar is an action bar: its actions run their commands
/// when clicked.
#[gpui::test]
fn the_status_bar_is_clickable(cx: &mut TestAppContext) {
    let (editor, cx) = setup("", None, |o| o.suggest.show_status_bar = true, cx);
    provide(cx, 3);
    cx.simulate_keystrokes("ctrl-space");
    draw(cx);
    let (rows, details) = painted(&editor, cx);
    assert!(details.is_none());
    // The status bar is a row under the rows: "Show More" on the right.
    let status = rows.bottom() + px(8.);
    cx.simulate_click(point(rows.right() - px(10.), status), Modifiers::none());
    draw(cx);
    assert!(editor.read_with(cx, |e, cx| e.is_suggestion_details_visible(cx)));
    // "Insert" on the left accepts the focused item.
    cx.simulate_click(point(rows.left() + px(10.), status), Modifiers::none());
    draw(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_suggest_visible()));
    assert_eq!(
        editor.read_with(cx, |e, cx| e.model().read(cx).text()),
        "item00"
    );
}

/// Signatures with long docs.
struct LongDocs;
impl SignatureHelpProvider for LongDocs {
    fn trigger_characters(&self) -> Vec<char> {
        vec!['(']
    }

    fn provide_signature_help(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: SignatureHelpTrigger,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<SignatureHelp>>> {
        let docs: String = (0..80).map(|i| format!("Paragraph {i}.\n\n")).collect();
        Task::ready(Ok(Some(SignatureHelp {
            signatures: vec![SignatureInformation {
                label: "f(a: u32)".into(),
                documentation: Some(Documentation::PlainText(docs.into())),
                parameters: vec![ParameterInformation {
                    label: ParameterLabel::Text("a: u32".into()),
                    documentation: None,
                }],
                active_parameter: None,
            }],
            active_signature: 0,
            active_parameter: 0,
        })))
    }
}

/// Monaco `ParameterHintsWidget.updateMaxHeight`: at most a quarter of the
/// editor (at least 250px), scrolling inside with the wheel.
#[gpui::test]
fn parameter_hints_have_a_maximum_height(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(60), None, |_| {}, cx);
    cx.update(|_, cx| register_signature_help_provider(cx, LanguageSelector::All, LongDocs));
    set_cursor(&editor, cx, 20, 0);
    cx.simulate_input("(");
    rest(cx, 120);
    assert!(editor.read_with(cx, |e, _| e.is_signature_help_visible()));
    let (_, widgets) = editor.read_with(cx, |e, _| e.widget_bounds_for_tests());
    let hints = widgets
        .iter()
        .rev()
        .find(|(_, kind)| *kind == EmbeddedKind::ContentWidget)
        .unwrap()
        .0;
    assert!(hints.size.height <= px(252.), "{hints:?}");
    assert!(hints.size.height > px(200.), "{hints:?}");
    // The wheel over it leaves the editor alone.
    let top = editor.read_with(cx, |e, _| e.scroll_top());
    wheel(cx, hints.center(), -100.);
    assert_eq!(editor.read_with(cx, |e, _| e.scroll_top()), top);
    assert!(editor.read_with(cx, |e, _| e.is_signature_help_visible()));
}

/// The suggest list's scroll position in pixels.
fn list_scroll_top(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Pixels {
    editor
        .read_with(cx, |e, cx| e.suggest_list_scroll_top_for_tests(cx))
        .unwrap()
}

/// Monaco's `ListView` scrolls by pixels: the wheel moves the list by its
/// pixels, the rows at the edges show in part, and moving the focus
/// reveals the focused row in full, as little as it takes.
#[gpui::test]
fn the_list_scrolls_by_pixels(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(30), None, |_| {}, cx);
    provide(cx, 40);
    set_cursor(&editor, cx, 2, 0);
    cx.simulate_keystrokes("ctrl-space");
    draw(cx);
    let (rows, _) = painted(&editor, cx);
    // Twelve rows at the default size.
    let item = rows.size.height / 12.;
    assert_eq!(list_scroll_top(&editor, cx), px(0.));
    // Less than a row: the first row shows in part.
    wheel(cx, rows.center(), -7.);
    assert_eq!(list_scroll_top(&editor, cx), px(7.));
    assert_eq!(first_row(&editor, cx), 0);
    // A row and some more.
    wheel(cx, rows.center(), -(f32::from(item) + 3.));
    assert_eq!(list_scroll_top(&editor, cx), (item + px(10.)).round());
    assert_eq!(first_row(&editor, cx), 1);
    // Back past the top: clamped.
    wheel(cx, rows.center(), 500.);
    assert_eq!(list_scroll_top(&editor, cx), px(0.));
    // The focused row (the first), partly scrolled out, comes back in full
    // when the focus moves to it; a row in view doesn't scroll.
    wheel(cx, rows.center(), -7.);
    cx.simulate_keystrokes("down");
    draw(cx);
    assert_eq!(list_scroll_top(&editor, cx), px(7.));
    cx.simulate_keystrokes("up");
    draw(cx);
    assert_eq!(list_scroll_top(&editor, cx), px(0.));
    // Down to the 13th row: just enough to show it at the bottom.
    for _ in 0..12 {
        cx.simulate_keystrokes("down");
    }
    draw(cx);
    let viewport = rows.size.height;
    assert_eq!(
        list_scroll_top(&editor, cx),
        (item * 13. - viewport).round()
    );
    // Monaco's `List.reveal` scrolls the list, which shows its scrollbar.
    assert!(editor.read_with(cx, |e, cx| e.suggest_scrollbars_revealed_for_tests(cx)));
    assert!(editor.read_with(cx, |e, _| e.is_suggest_visible()));
}

/// Monaco `ScrollbarVisibilityController`: the list's scrollbar fades in
/// over 100ms while the mouse is over the list and out over 800ms after it
/// leaves, repainting only while it fades.
#[gpui::test]
fn the_scrollbars_fade(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(30), None, |_| {}, cx);
    provide(cx, 40);
    set_cursor(&editor, cx, 2, 0);
    cx.simulate_keystrokes("ctrl-space");
    draw(cx);
    let opacities = |cx: &mut VisualTestContext| {
        editor
            .read_with(cx, |e, cx| e.suggest_scrollbar_opacities_for_tests(cx))
            .unwrap()
    };
    // Hidden, and not repainting.
    assert_eq!(opacities(cx), ([0., 0.], false));
    let (rows, _) = painted(&editor, cx);
    cx.simulate_mouse_move(rows.center(), None, Modifiers::none());
    draw(cx);
    // Fading in.
    let ([list, _], fading) = opacities(cx);
    assert!(fading && list < 0.5, "{list}");
    rest(cx, 50);
    let ([list, _], fading) = opacities(cx);
    assert!(fading && (0.3..0.7).contains(&list), "{list}");
    rest(cx, 60);
    assert_eq!(opacities(cx), ([1., 0.], false));
    // The mouse leaves: fading out.
    cx.simulate_mouse_move(point(px(5.), px(5.)), None, Modifiers::none());
    draw(cx);
    let ([list, _], fading) = opacities(cx);
    assert!(fading && list > 0.9, "{list}");
    rest(cx, 400);
    let ([list, _], fading) = opacities(cx);
    assert!(fading && (0.3..0.7).contains(&list), "{list}");
    rest(cx, 500);
    assert_eq!(opacities(cx), ([0., 0.], false));
    // Back in mid-fade: it fades in from where it was.
    cx.simulate_mouse_move(rows.center(), None, Modifiers::none());
    draw(cx);
    rest(cx, 150);
    cx.simulate_mouse_move(point(px(5.), px(5.)), None, Modifiers::none());
    draw(cx);
    rest(cx, 400);
    cx.simulate_mouse_move(rows.center(), None, Modifiers::none());
    draw(cx);
    let ([from, _], _) = opacities(cx);
    assert!((0.3..0.7).contains(&from), "{from}");
    rest(cx, 25);
    let ([list, _], fading) = opacities(cx);
    assert!(fading && list > from && list < 1., "{from} {list}");
}

/// Items whose docs grow with their index: `count` of `item0`… with
/// `index + 1` paragraphs each.
struct GrowingDocs(usize);
impl CompletionProvider for GrowingDocs {
    fn provide_completions(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: CompletionTrigger,
        _: &mut App,
    ) -> Task<anyhow::Result<CompletionList>> {
        let items = (0..self.0)
            .map(|i| CompletionItem {
                detail: Some(format!("fn item{i}()").into()),
                documentation: Some(
                    (0..=i)
                        .map(|p| format!("Paragraph {p} of item {i}."))
                        .collect::<Vec<_>>()
                        .join("\n\n")
                        .into(),
                ),
                ..CompletionItem::new(format!("item{i}"), CompletionItemKind::Method)
            })
            .collect();
        Task::ready(Ok(CompletionList {
            items,
            incomplete: false,
        }))
    }
}

/// Monaco measures the details' type and docs before placing the panel:
/// it fits its content in the frame the content changes, not a frame
/// later.
#[gpui::test]
fn the_details_fit_their_content_in_the_same_frame(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(30), None, |_| {}, cx);
    cx.update(|_, cx| register_completion_provider(cx, LanguageSelector::All, GrowingDocs(4)));
    set_cursor(&editor, cx, 2, 0);
    cx.simulate_keystrokes("ctrl-space");
    draw(cx);
    let fitted = |cx: &mut VisualTestContext| {
        let (side, bounds) = editor
            .read_with(cx, |e, cx| e.suggest_details_placement_for_tests(cx))
            .expect("details shown");
        assert_eq!(side, "east");
        let content = editor
            .read_with(cx, |e, cx| e.suggest_details_content_height_for_tests(cx))
            .unwrap();
        // Inside its 1px border.
        assert!(
            (bounds.size.height - px(2.) - content).abs() < px(0.5),
            "{bounds:?} {content:?}"
        );
        content
    };
    // One frame after Ctrl+Space shows the details.
    cx.simulate_keystrokes("ctrl-space");
    draw(cx);
    let mut last = fitted(cx);
    // Each item's docs are taller: one frame each.
    for _ in 0..3 {
        cx.simulate_keystrokes("down");
        draw(cx);
        let height = fitted(cx);
        assert!(height > last, "{last:?} {height:?}");
        last = height;
    }
    // And shorter again.
    cx.simulate_keystrokes("up");
    draw(cx);
    assert!(fitted(cx) < last);
}

/// In Monaco the list and the details panel are separate widgets: the
/// empty corner of the box around both lets clicks through to the editor.
#[gpui::test]
fn the_empty_corners_let_clicks_through(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(30), None, |_| {}, cx);
    provide(cx, 40);
    set_cursor(&editor, cx, 2, 0);
    cx.simulate_keystrokes("ctrl-space");
    draw(cx);
    cx.simulate_keystrokes("ctrl-space");
    draw(cx);
    let (rows, details) = painted(&editor, cx);
    let details = details.expect("details shown");
    // East of the list and shorter than it: the corner below the details.
    assert!(details.left() >= rows.right());
    assert!(details.bottom() + px(40.) < rows.bottom());
    let corner = point(details.center().x, details.bottom() + px(30.));
    let (_, widgets) = editor.read_with(cx, |e, _| e.widget_bounds_for_tests());
    assert!(
        !widgets.iter().any(|(bounds, _)| bounds.contains(&corner)),
        "{widgets:?}"
    );
    // A click on the details doesn't reach the editor.
    cx.simulate_click(details.center(), Modifiers::none());
    draw(cx);
    assert_eq!(cursor_row(&editor, cx), 2);
    assert!(editor.read_with(cx, |e, _| e.is_suggest_visible()));
    // A click in the corner lands in the text, below the cursor's line.
    cx.simulate_click(corner, Modifiers::none());
    draw(cx);
    assert!(cursor_row(&editor, cx) > 2);
}

fn cursor_row(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> u32 {
    editor.read_with(cx, |e, _| e.selections()[0].head.row)
}
