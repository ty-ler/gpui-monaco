mod providers;
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
        if std::env::var_os("PLAYGROUND_DEMO").is_some() {
            add_demo_decorations(&editor, cx);
            add_demo_widgets(&editor, cx);
        }
        if std::env::var_os("PLAYGROUND_PROVIDERS").is_some() {
            providers::register(cx);
            providers::add_markers(&editor, cx);
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

/// A small label view for demo widgets.
struct DemoLabel {
    text: gpui::SharedString,
    background: gpui::Hsla,
}

impl Render for DemoLabel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .px_2()
            .py_1()
            .rounded_sm()
            .bg(self.background)
            .text_color(gpui::white())
            .text_sm()
            .child(self.text.clone())
    }
}

/// A view zone, a content widget and an overlay widget.
fn add_demo_widgets(editor: &Entity<CodeEditor>, cx: &mut App) {
    use gpui::{hsla, rgb};
    use gpui_monaco::{
        ContentWidget, ContentWidgetPreference, OverlayPosition, OverlayWidget, ViewZone,
        ZoneHeight,
    };
    let zone = cx.new(|_| DemoLabel {
        text: "A view zone between lines 4 and 5 — any GPUI view".into(),
        background: hsla(0.6, 0.5, 0.3, 1.),
    });
    let content = cx.new(|_| DemoLabel {
        text: "content widget".into(),
        background: rgb(0x0e639c).into(),
    });
    let overlay = cx.new(|_| DemoLabel {
        text: "overlay widget".into(),
        background: rgb(0x6c3483).into(),
    });
    editor.update(cx, |editor, cx| {
        editor.add_view_zone(
            ViewZone {
                after_line: 4,
                height: ZoneHeight::Lines(2.),
                view: zone.into(),
            },
            cx,
        );
        editor.add_content_widget(
            ContentWidget {
                position: gpui_monaco::text::PointUtf16::new(16, 4),
                preference: vec![
                    ContentWidgetPreference::Above,
                    ContentWidgetPreference::Below,
                ],
                view: content.into(),
            },
            cx,
        );
        editor.add_overlay_widget(
            OverlayWidget {
                position: OverlayPosition::TopRight,
                view: overlay.into(),
            },
            cx,
        );
    });
}

/// Decorations showing off each decoration style.
fn add_demo_decorations(editor: &Entity<CodeEditor>, cx: &mut App) {
    use gpui::{hsla, rgb};
    use gpui_monaco::{
        DecorationOptions, DecorationUnderline, InjectedText, OverviewRulerLane, TokenStyle,
    };
    let model = editor.read(cx).model().clone();
    model.update(cx, |model, cx| {
        let text = model.buffer().rope().to_string();
        let find = |needle: &str| {
            let start = text.find(needle).unwrap_or(0);
            start..start + needle.len()
        };
        let error = rgb(0xf14c4c).into();
        let warning = rgb(0xcca700).into();
        model.delta_decorations(
            None,
            &[],
            vec![
                (
                    find("transform_offset"),
                    DecorationOptions {
                        underline: Some(DecorationUnderline {
                            color: error,
                            wavy: true,
                        }),
                        overview_ruler: Some((error, OverviewRulerLane::Right)),
                        hover_message: Some("demo error".into()),
                        glyph: Some(("●".into(), error)),
                        ..Default::default()
                    },
                ),
                (
                    find("pub struct Patch"),
                    DecorationOptions {
                        background: Some(hsla(0.15, 0.8, 0.5, 0.15)),
                        is_whole_line: true,
                        gutter_bar: Some(rgb(0x2ea043).into()),
                        line_number_color: Some(rgb(0x2ea043).into()),
                        overview_ruler: Some((rgb(0x2ea043).into(), OverviewRulerLane::Left)),
                        ..Default::default()
                    },
                ),
                (
                    find("fn delta"),
                    DecorationOptions {
                        border: Some(warning),
                        underline: Some(DecorationUnderline {
                            color: warning,
                            wavy: false,
                        }),
                        after: Some(InjectedText {
                            content: " ← inline hint ".into(),
                            style: TokenStyle {
                                foreground: Some(hsla(0., 0., 0.6, 1.)),
                                italic: true,
                                ..Default::default()
                            },
                            background: Some(hsla(0., 0., 0.5, 0.15)),
                        }),
                        ..Default::default()
                    },
                ),
                (
                    find("Range<usize>,"),
                    DecorationOptions {
                        text: Some(TokenStyle {
                            foreground: Some(rgb(0xff8800).into()),
                            bold: true,
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                ),
            ],
            cx,
        );
    });
}
