mod screenshot;

use gpui::{
    App, AppContext as _, Bounds, Context, Entity, Focusable, Window, WindowBounds, WindowOptions,
    div, prelude::*, px, size,
};
use gpui_monaco::{CodeEditor, TextModel};

const SAMPLE: &str = include_str!("../samples/sample.rs");

struct Playground {
    editor: Entity<CodeEditor>,
}

impl Playground {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let text = match std::env::var("PLAYGROUND_FILE") {
            Ok(path) => std::fs::read_to_string(&path).unwrap_or_else(|e| format!("{path}: {e}")),
            Err(_) => SAMPLE.to_string(),
        };
        let model = cx.new(|_| TextModel::new(&text).with_language("rust"));
        let editor = cx.new(|cx| CodeEditor::new(model, window, cx));
        window.focus(&editor.focus_handle(cx), cx);
        Self { editor }
    }
}

impl Render for Playground {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.editor.clone())
    }
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        gpui_monaco::init(cx);
        let bounds = Bounds::centered(None, size(px(1000.), px(700.)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| Playground::new(window, cx)),
            )
            .expect("failed to open window");
        cx.activate(true);
        screenshot::capture_if_requested(window.into(), cx);
    });
}
