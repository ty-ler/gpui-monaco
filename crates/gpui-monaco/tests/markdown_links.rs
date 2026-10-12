//! Links clicked in markdown go through Monaco's opener service: with no
//! link opener taking them, a link into the focused editor's model selects
//! its `#L<line>,<col>` fragment and centers it (the standalone
//! `doOpenEditor`), and a link to another model goes to the editor openers
//! (Monaco's `EditorOpener`, `openCodeEditor`).

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    App, AppContext as _, Entity, Focusable as _, Modifiers, TestAppContext, VisualTestContext,
    point, px,
};
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, TextModel};

/// A hover showing whatever markdown the test sets.
struct MarkdownHover(Rc<RefCell<MarkdownString>>);
impl HoverProvider for MarkdownHover {
    fn provide_hover(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: &mut App,
    ) -> gpui::Task<anyhow::Result<Option<Hover>>> {
        gpui::Task::ready(Ok(Some(Hover {
            contents: vec![self.0.borrow().clone()],
            range: None,
            ..Default::default()
        })))
    }
}

/// A uri and a range an editor opener got.
type Opened = (String, Option<Range<PointUtf16>>);

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

fn setup(
    cx: &mut TestAppContext,
) -> (
    Entity<CodeEditor>,
    Rc<RefCell<MarkdownString>>,
    &mut VisualTestContext,
) {
    cx.update(gpui_monaco::init);
    let content = Rc::new(RefCell::new(MarkdownString::default()));
    let provider = MarkdownHover(content.clone());
    cx.update(|cx| register_hover_provider(cx, LanguageSelector::All, provider));
    let text: String = (0..200).map(|i| format!("line {i}\n")).collect();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_uri("file:///test.rs"));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.read(cx).focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, content, cx)
}

fn paint(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

/// Shows a hover with `markdown` (a single link) and clicks its text.
fn click_hover_link(
    editor: &Entity<CodeEditor>,
    content: &Rc<RefCell<MarkdownString>>,
    markdown: &str,
    cx: &mut VisualTestContext,
) {
    *content.borrow_mut() = MarkdownString::new(markdown);
    editor.update(cx, |e, cx| {
        e.hide_hover(cx);
        e.show_hover_at_cursor(cx);
    });
    cx.run_until_parked();
    paint(cx);
    paint(cx);
    let part = cx.debug_bounds("hover-part-0").expect("hover part");
    // The contents' `padding: 4px 8px`, then the link's first letters.
    let at = part.origin + point(px(12.), px(10.));
    cx.simulate_click(at, Modifiers::default());
    cx.run_until_parked();
    paint(cx);
}

/// Monaco `EditorOpener` with the standalone code editor service: a link
/// into the editor's own model selects the fragment and centers it.
#[gpui::test]
fn markdown_links_into_the_model_move_the_editor(cx: &mut TestAppContext) {
    let (editor, content, cx) = setup(cx);
    click_hover_link(&editor, &content, "[line 120](file:///test.rs#L120,3)", cx);
    let selection = editor.read_with(cx, |e, _| e.selection());
    assert_eq!((selection.anchor, selection.head), (p(119, 2), p(119, 2)));
    let scroll_top = editor.read_with(cx, |e, _| e.scroll_position().y);
    assert!(scroll_top > px(0.), "centered on line 120: {scroll_top:?}");

    // A range: `#L<line>,<col>-L<line>,<col>`.
    click_hover_link(&editor, &content, "[range](file:///test.rs#L3,1-L4,5)", cx);
    let selection = editor.read_with(cx, |e, _| e.selection());
    assert_eq!((selection.anchor, selection.head), (p(2, 0), p(3, 4)));
}

/// Links to other models reach the editor openers, with the fragment's
/// position, and link openers still come first.
#[gpui::test]
fn markdown_links_to_other_models_reach_the_editor_openers(cx: &mut TestAppContext) {
    let (editor, content, cx) = setup(cx);
    let opened: Rc<RefCell<Vec<Opened>>> = Default::default();
    let sink = opened.clone();
    let _editor_opener = cx.update(|_, cx| {
        gpui_monaco::register_editor_opener(cx, move |_, uri, selection, _| {
            sink.borrow_mut().push((uri.to_string(), selection));
            true
        })
    });
    click_hover_link(&editor, &content, "[other](file:///other.rs#L10,5)", cx);
    assert_eq!(
        *opened.borrow(),
        [("file:///other.rs".to_string(), Some(p(9, 4)..p(9, 4)))]
    );
    // The editor didn't move.
    let selection = editor.read_with(cx, |e, _| e.selection());
    assert_eq!(selection.head, p(0, 0));

    let links = Rc::new(RefCell::new(Vec::new()));
    let sink = links.clone();
    let _link_opener = cx.update(|_, cx| {
        gpui_monaco::register_link_opener(cx, move |url, _| {
            sink.borrow_mut().push(url.to_string());
            true
        })
    });
    click_hover_link(&editor, &content, "[mine](file:///test.rs#L50)", cx);
    assert_eq!(*links.borrow(), ["file:///test.rs#L50"]);
    let selection = editor.read_with(cx, |e, _| e.selection());
    assert_eq!(selection.head, p(0, 0));
}
