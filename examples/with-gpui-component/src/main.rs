//! gpui-monaco's `CodeEditor` in a gpui-component window: a gpui-component
//! `Root` with a toolbar of gpui-component buttons above the editor. Both
//! crates build against the same `gpui-pre` snapshot, so their types line up.

use gpui::{
    App, AppContext as _, Bounds, Context, Entity, Focusable as _, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Window, WindowBounds, WindowOptions, div, px, size,
};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::{ActiveTheme as _, Root, h_flex, v_flex};
use gpui_monaco::{CodeEditor, TextModel};

const SAMPLE: &str = "fn main() {\n    let greeting = \"Hello from gpui-monaco\";\n    println!(\"{greeting}\");\n}\n";

struct Demo {
    editor: Entity<CodeEditor>,
    status: SharedString,
}

impl Demo {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let model = cx.new(|_| TextModel::new(SAMPLE).with_language("rust"));
        let editor = cx.new(|cx| CodeEditor::new(model, window, cx));
        window.focus(&editor.read(cx).focus_handle(cx), cx);
        Self {
            editor,
            status: "Ready".into(),
        }
    }

    fn count_lines(&mut self, cx: &mut Context<Self>) {
        let lines = self.editor.read(cx).model().read(cx).line_count();
        self.status = format!("{lines} lines").into();
        cx.notify();
    }

    fn undo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor
            .update(cx, |editor, cx| editor.run_action("undo", window, cx));
    }
}

impl Render for Demo {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                h_flex()
                    .gap_2()
                    .p_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        Button::new("count")
                            .primary()
                            .label("Count lines")
                            .on_click(cx.listener(|this, _, _, cx| this.count_lines(cx))),
                    )
                    .child(
                        Button::new("undo")
                            .label("Undo")
                            .on_click(cx.listener(|this, _, window, cx| this.undo(window, cx))),
                    )
                    .child(div().text_sm().child(self.status.clone())),
            )
            .child(div().flex_1().min_h_0().child(self.editor.clone()))
    }
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        // Both libraries initialize their actions, keymaps and globals.
        gpui_component::init(cx);
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
        let bounds = Bounds::centered(None, size(px(900.), px(600.)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |window, cx| {
                    let demo = cx.new(|cx| Demo::new(window, cx));
                    cx.new(|cx| Root::new(demo, window, cx))
                },
            )
            .expect("failed to open window");
        cx.activate(true);
        screenshot(window.into(), cx);
    });
}

/// `EXAMPLE_SCREENSHOT=out.png` with `--features screenshot`.
#[cfg(feature = "screenshot")]
fn screenshot(window: gpui::AnyWindowHandle, cx: &mut App) {
    let Ok(path) = std::env::var("EXAMPLE_SCREENSHOT") else {
        return;
    };
    cx.spawn(async move |cx| {
        cx.background_executor()
            .timer(std::time::Duration::from_millis(800))
            .await;
        let result = cx.update_window(window, |_, window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
            window.render_to_image()
        });
        match result {
            Ok(Ok(image)) => {
                image.save(&path).expect("failed to save screenshot");
                eprintln!("saved screenshot to {path}");
            }
            Ok(Err(error)) => eprintln!("render_to_image failed: {error:#}"),
            Err(error) => eprintln!("window gone: {error:#}"),
        }
        cx.update(|cx| cx.quit());
    })
    .detach();
}

#[cfg(not(feature = "screenshot"))]
fn screenshot(_: gpui::AnyWindowHandle, _: &mut App) {}
