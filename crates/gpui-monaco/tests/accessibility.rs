//! Accessibility: tab focus mode (Monaco `toggleTabFocusMode`).

use gpui::{
    AppContext, Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement, Render,
    Styled, TestAppContext, Window, div,
};
use gpui_monaco::CodeEditor;
use gpui_monaco::actions::ToggleTabFocusMode;

struct TwoEditors {
    first: Entity<CodeEditor>,
    second: Entity<CodeEditor>,
}

impl Render for TwoEditors {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().h_40().child(self.first.clone()))
            .child(div().h_40().child(self.second.clone()))
    }
}

#[gpui::test]
fn tab_moves_focus_in_tab_focus_mode(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let (view, cx) = cx.add_window_view(|window, cx| TwoEditors {
        first: cx.new(|cx| CodeEditor::with_text("one", window, cx)),
        second: cx.new(|cx| CodeEditor::with_text("two", window, cx)),
    });
    let (first, second) = view.read_with(cx, |v, _| (v.first.clone(), v.second.clone()));
    let handle = |e: &Entity<CodeEditor>, cx: &mut gpui::VisualTestContext| -> FocusHandle {
        cx.update(|_, cx| e.focus_handle(cx))
    };
    let first_handle = handle(&first, cx);
    cx.update(|window, cx| window.focus(&first_handle, cx));
    cx.run_until_parked();

    // Normally Tab indents.
    cx.simulate_keystrokes("tab");
    assert_eq!(
        first.read_with(cx, |e, cx| e.model().read(cx).text()),
        "    one"
    );

    cx.dispatch_action(ToggleTabFocusMode);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert!(cx.update(|_, cx| gpui_monaco::tab_focus_mode(cx)));
    cx.simulate_keystrokes("tab");
    let second_handle = handle(&second, cx);
    assert!(cx.update(|window, _| second_handle.is_focused(window)));
    assert_eq!(
        first.read_with(cx, |e, cx| e.model().read(cx).text()),
        "    one"
    );
    cx.simulate_keystrokes("shift-tab");
    assert!(cx.update(|window, _| first_handle.is_focused(window)));
}
