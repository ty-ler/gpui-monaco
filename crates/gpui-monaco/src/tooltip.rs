//! Hover titles on the editor's buttons (Monaco's `title` attributes, shown
//! through its hover service), in the editor widget colors.

use gpui::{
    AnyView, App, AppContext as _, Hsla, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Window, div, px,
};

use crate::theme::EditorColors;

struct Tooltip {
    text: SharedString,
    background: Hsla,
    foreground: Hsla,
    border: Hsla,
}

#[cfg(test)]
thread_local! {
    /// The texts of tooltips rendered, for tests.
    static RENDERED: std::cell::RefCell<Vec<SharedString>> = const { std::cell::RefCell::new(Vec::new()) };
}

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        RENDERED.with(|r| r.borrow_mut().push(self.text.clone()));
        div()
            .px(px(8.))
            .py(px(4.))
            .rounded(px(3.))
            .border_1()
            .border_color(self.border)
            .bg(self.background)
            .text_color(self.foreground)
            .text_size(px(12.))
            .whitespace_nowrap()
            .child(self.text.clone())
    }
}

/// A tooltip builder showing `text`, for `.tooltip(…)`.
pub(crate) fn tooltip(
    text: impl Into<SharedString>,
    colors: &EditorColors,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text = text.into();
    let (background, foreground, border) = (
        colors.widget_background,
        colors.widget_foreground,
        colors.widget_border,
    );
    move |_, cx| {
        let text = text.clone();
        cx.new(|_| Tooltip {
            text,
            background,
            foreground,
            border,
        })
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{InteractiveElement as _, StatefulInteractiveElement as _, TestAppContext, point};

    struct Button;

    impl Render for Button {
        fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .child(div().id("button").w(px(40.)).h(px(20.)).tooltip(tooltip(
                    "Revert Block",
                    &crate::theme::Theme::vs_dark().colors,
                )))
        }
    }

    #[gpui::test]
    fn hovering_shows_the_title(cx: &mut TestAppContext) {
        let (_, cx) = cx.add_window_view(|_, _| Button);
        cx.run_until_parked();
        cx.simulate_mouse_move(point(px(10.), px(10.)), None, gpui::Modifiers::none());
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(1));
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert!(RENDERED.with(|r| r.borrow().iter().any(|t| t == "Revert Block")));
    }
}
