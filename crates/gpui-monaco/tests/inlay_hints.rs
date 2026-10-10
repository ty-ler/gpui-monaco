//! Inlay hints (Monaco `contrib/inlayHints`) with a test provider.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, AppContext as _, Entity, Focusable, Modifiers, Task, TestAppContext, VisualTestContext,
};
use gpui_monaco::options::InlayHintsMode;
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, TextModel};

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
                    tooltip: None,
                    padding_left: false,
                    padding_right: false,
                });
            }
        }
        Task::ready(Ok(hints))
    }
}

fn setup(types: Types, cx: &mut TestAppContext) -> (Entity<CodeEditor>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_inlay_hints_provider(cx, LanguageSelector::All, types);
    });
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new("let x = 1;\nlet yy = 2;\n"));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    settle(cx);
    (editor, cx)
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
