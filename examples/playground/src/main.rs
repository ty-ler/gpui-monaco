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
        let (text, language) = match std::env::var("PLAYGROUND_FILE") {
            Ok(path) => {
                let text =
                    std::fs::read_to_string(&path).unwrap_or_else(|e| format!("{path}: {e}"));
                let language = gpui_monaco::language_registry(cx)
                    .language_for_path(path.as_ref())
                    .map_or_else(|| "plaintext".to_string(), |l| l.id.to_string());
                (text, language)
            }
            Err(_) => (SAMPLE.to_string(), "rust".to_string()),
        };
        let model = cx.new(|_| TextModel::new(&text).with_language(language));
        let editor = cx.new(|cx| CodeEditor::new(model, window, cx));
        if let Ok(wrap) = std::env::var("PLAYGROUND_WRAP") {
            editor.update(cx, |editor, cx| {
                let mut options = editor.options().clone();
                match wrap.parse::<u32>() {
                    Ok(column) => {
                        options.word_wrap = gpui_monaco::options::WordWrap::WordWrapColumn;
                        options.word_wrap_column = column;
                    }
                    Err(_) => options.word_wrap = gpui_monaco::options::WordWrap::On,
                }
                editor.update_options(options, cx);
            });
        }
        if let Ok(theme) = std::env::var("PLAYGROUND_THEME") {
            let theme = match theme.as_str() {
                "vs" => gpui_monaco::Theme::vs(),
                "vs-dark" => gpui_monaco::Theme::vs_dark(),
                path => gpui_monaco::Theme::from_vscode_file(path.as_ref())
                    .unwrap_or_else(|e| panic!("failed to load theme {path}: {e}")),
            };
            editor.update(cx, |editor, cx| {
                editor.set_theme(std::sync::Arc::new(theme), cx)
            });
        }
        window.focus(&editor.focus_handle(cx), cx);
        if std::env::var_os("PLAYGROUND_DEBUG").is_some() {
            cx.subscribe(
                &editor,
                |_, editor, event: &gpui_monaco::CodeEditorEvent, cx| {
                    if *event == gpui_monaco::CodeEditorEvent::SelectionsChanged {
                        eprintln!("selections: {:?}", editor.read(cx).selections());
                    }
                },
            )
            .detach();
        }
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
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
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
