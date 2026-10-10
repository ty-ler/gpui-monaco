mod perf;
mod providers;
mod screenshot;

use std::sync::Arc;

use gpui::{
    App, AppContext as _, Bounds, Context, Entity, Focusable, PathPromptOptions, SharedString,
    Window, WindowBounds, WindowOptions, div, prelude::*, px, size,
};
use gpui_monaco::options::{LineNumbers, RenderWhitespace, Ruler, WordWrap};
use gpui_monaco::text::LineEnding;
use gpui_monaco::{
    CodeEditor, CodeEditorEvent, DecorationId, DecorationOptions, EditorColors, EditorOptions,
    MouseTargetKind, TextModel, Theme,
};
use gpui_monaco_lsp::{LanguageServerBinary, LspClient, LspClientEvent, LspClientOptions};

const SAMPLE: &str = include_str!("../samples/sample.rs");

/// Bundled VS Code themes (MIT, see `themes/README.md`).
const THEME_FILES: &[(&str, &str)] = &[
    ("dark_vs.json", include_str!("../themes/dark_vs.json")),
    ("dark_plus.json", include_str!("../themes/dark_plus.json")),
    (
        "dark_modern.json",
        include_str!("../themes/dark_modern.json"),
    ),
    ("light_vs.json", include_str!("../themes/light_vs.json")),
    ("light_plus.json", include_str!("../themes/light_plus.json")),
];

fn themes() -> Vec<(&'static str, Arc<Theme>)> {
    let vscode = |file: &str| {
        let json = THEME_FILES
            .iter()
            .find(|(name, _)| *name == file)
            .map(|(_, json)| *json)
            .unwrap_or("{}");
        Theme::from_vscode_json_with_includes(json, &|include| {
            let include = include.trim_start_matches("./");
            THEME_FILES
                .iter()
                .find(|(name, _)| *name == include)
                .map(|(_, json)| json.to_string())
        })
        .expect("bundled theme parses")
    };
    vec![
        ("vs-dark", Arc::new(Theme::vs_dark())),
        ("vs", Arc::new(Theme::vs())),
        ("Dark Modern", Arc::new(vscode("dark_modern.json"))),
        ("Dark+", Arc::new(vscode("dark_plus.json"))),
        ("Light+", Arc::new(vscode("light_plus.json"))),
    ]
}

/// A `file://` uri for `path` (made absolute).
fn file_uri(path: &str) -> String {
    let path = std::path::absolute(path).unwrap_or_else(|_| path.into());
    let path = path.to_string_lossy().replace('\\', "/");
    let mut uri = String::from("file://");
    if !path.starts_with('/') {
        uri.push('/');
    }
    for c in path.chars() {
        match c {
            ' ' => uri.push_str("%20"),
            '#' => uri.push_str("%23"),
            '?' => uri.push_str("%3F"),
            '%' => uri.push_str("%25"),
            _ => uri.push(c),
        }
    }
    uri
}

/// Starts the `PLAYGROUND_LSP` server (a command line, e.g. `rust-analyzer`
/// or `typescript-language-server --stdio`) for the project around `path`
/// and attaches `model` to it.
fn start_language_server(
    command: &str,
    path: &str,
    language: SharedString,
    model: &Entity<TextModel>,
    cx: &mut Context<Playground>,
) {
    let mut words = command.split_whitespace();
    let Some(program) = words.next() else {
        return;
    };
    let file = std::path::absolute(path).unwrap_or_else(|_| path.into());
    // The nearest directory with a project marker, else the file's.
    let root = file
        .ancestors()
        .skip(1)
        .find(|dir| {
            [
                "Cargo.toml",
                "package.json",
                "pyproject.toml",
                "go.mod",
                ".git",
            ]
            .iter()
            .any(|marker| dir.join(marker).exists())
        })
        .or_else(|| file.parent())
        .map(|dir| dir.to_path_buf());
    let binary = LanguageServerBinary {
        path: program.into(),
        args: words.map(str::to_string).collect(),
        cwd: root.clone(),
        ..Default::default()
    };
    let options = LspClientOptions {
        name: program.to_string().into(),
        languages: vec![language],
        root_uri: root.map(|root| file_uri(&root.to_string_lossy())),
        ..Default::default()
    };
    let start = LspClient::start(binary, options, cx);
    let model = model.clone();
    cx.spawn(async move |this, cx| {
        let client = match start.await {
            Ok(client) => client,
            Err(err) => {
                eprintln!("language server: {err:#}");
                return;
            }
        };
        this.update(cx, |this, cx| {
            cx.subscribe(&client, |_, _, event: &LspClientEvent, _| {
                if let LspClientEvent::ShowMessage { message, .. } = event {
                    eprintln!("language server: {message}");
                }
            })
            .detach();
            if let Err(err) = client.update(cx, |client, cx| client.attach(&model, cx)) {
                eprintln!("language server: {err:#}");
            }
            this.lsp = Some(client);
        })
        .ok();
    })
    .detach();
}

struct Playground {
    editor: Entity<CodeEditor>,
    themes: Vec<(&'static str, Arc<Theme>)>,
    theme: usize,
    languages: Vec<SharedString>,
    file_name: SharedString,
    /// Breakpoint decorations by row.
    breakpoints: Vec<(u32, DecorationId)>,
    /// The language server from `PLAYGROUND_LSP`, once it's running.
    lsp: Option<Entity<LspClient>>,
}

impl Playground {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (text, language, file_name): (String, SharedString, SharedString) =
            match std::env::var("PLAYGROUND_FILE") {
                Ok(path) => {
                    let text =
                        std::fs::read_to_string(&path).unwrap_or_else(|e| format!("{path}: {e}"));
                    let language = language_for_path(&path, cx);
                    (text, language, path.into())
                }
                Err(_) => (SAMPLE.to_string(), "rust".into(), "sample.rs".into()),
            };
        let lsp_command = std::env::var("PLAYGROUND_LSP").ok();
        let uri = file_uri(file_name.as_ref());
        let model = cx.new(|_| {
            TextModel::new(&text)
                .with_language(language.clone())
                .with_uri(uri)
        });
        if let Some(command) = &lsp_command {
            start_language_server(command, file_name.as_ref(), language, &model, cx);
        }
        let editor = cx.new(|cx| CodeEditor::new(model, window, cx));
        if let Ok(wrap) = std::env::var("PLAYGROUND_WRAP") {
            editor.update(cx, |editor, cx| {
                let mut options = editor.options().clone();
                match wrap.parse::<u32>() {
                    Ok(column) => {
                        options.word_wrap = WordWrap::WordWrapColumn;
                        options.word_wrap_column = column;
                    }
                    Err(_) => options.word_wrap = WordWrap::On,
                }
                editor.update_options(options, cx);
            });
        }
        if std::env::var_os("PLAYGROUND_WHITESPACE").is_some() {
            editor.update(cx, |editor, cx| {
                let mut options = editor.options().clone();
                options.render_whitespace = RenderWhitespace::All;
                options.rulers = vec![Ruler {
                    column: 40,
                    color: None,
                }];
                editor.update_options(options, cx);
            });
        }
        // Demos are on unless disabled (`PLAYGROUND_PLAIN=1`).
        let plain = std::env::var_os("PLAYGROUND_PLAIN").is_some();
        if std::env::var_os("PLAYGROUND_DEMO").is_some() {
            add_demo_decorations(&editor, cx);
            add_demo_widgets(&editor, cx);
        }
        // A real language server replaces the mock providers.
        if lsp_command.is_none() && (!plain || std::env::var_os("PLAYGROUND_PROVIDERS").is_some()) {
            providers::register(cx);
            providers::add_markers(&editor, cx);
        }
        let themes = themes();
        let mut theme = 0;
        if let Ok(name) = std::env::var("PLAYGROUND_THEME") {
            match themes.iter().position(|(n, _)| *n == name) {
                Some(ix) => theme = ix,
                None => {
                    let loaded = Theme::from_vscode_file(name.as_ref())
                        .unwrap_or_else(|e| panic!("failed to load theme {name}: {e}"));
                    editor.update(cx, |editor, cx| editor.set_theme(Arc::new(loaded), cx));
                    theme = usize::MAX;
                }
            }
        }
        if theme != usize::MAX {
            let selected = themes[theme].1.clone();
            editor.update(cx, |editor, cx| editor.set_theme(selected, cx));
        }
        window.focus(&editor.focus_handle(cx), cx);
        if std::env::var_os("PLAYGROUND_DEBUG").is_some() {
            cx.subscribe(&editor, |_, editor, event: &CodeEditorEvent, cx| {
                if matches!(event, CodeEditorEvent::SelectionsChanged(_)) {
                    eprintln!("selections: {:?}", editor.read(cx).selections());
                }
            })
            .detach();
        }
        cx.subscribe(&editor, |this, editor, event: &CodeEditorEvent, cx| {
            // Breakpoints: clicking the glyph margin toggles a red dot.
            if let CodeEditorEvent::MouseDown(mouse) = event
                && mouse.target.kind == MouseTargetKind::GutterGlyphMargin
                && let Some(position) = mouse.target.position
            {
                this.toggle_breakpoint(&editor, position.row, cx);
            }
            cx.notify();
        })
        .detach();
        let languages = gpui_monaco::language_registry(cx)
            .languages()
            .iter()
            .filter(|l| !l.hidden)
            .map(|l| SharedString::from(l.id.to_string()))
            .collect();
        Self {
            editor,
            themes,
            theme: theme.min(usize::MAX - 1),
            languages,
            file_name,
            breakpoints: Vec::new(),
            lsp: None,
        }
    }

    fn toggle_breakpoint(&mut self, editor: &Entity<CodeEditor>, row: u32, cx: &mut Context<Self>) {
        let model = editor.read(cx).model().clone();
        if let Some(ix) = self.breakpoints.iter().position(|(r, _)| *r == row) {
            let (_, id) = self.breakpoints.remove(ix);
            model.update(cx, |model, cx| {
                model.delta_decorations(None, &[id], Vec::new(), cx)
            });
            return;
        }
        let offset = model
            .read(cx)
            .offset_at(gpui_monaco::text::PointUtf16::new(row, 0));
        let ids = model.update(cx, |model, cx| {
            model.delta_decorations(
                None,
                &[],
                vec![(
                    offset..offset,
                    DecorationOptions {
                        is_whole_line: true,
                        glyph: Some(("●".into(), gpui::rgb(0xe51400).into())),
                        ..Default::default()
                    },
                )],
                cx,
            )
        });
        self.breakpoints.push((row, ids[0]));
    }

    fn open_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let Ok(text) = std::fs::read_to_string(&path) else {
                return;
            };
            this.update_in(cx, |this, window, cx| {
                let path = path.to_string_lossy().to_string();
                let language = language_for_path(&path, cx);
                let uri = file_uri(&path);
                let model = cx.new(|_| TextModel::new(&text).with_language(language).with_uri(uri));
                if let Some(lsp) = &this.lsp {
                    lsp.update(cx, |lsp, cx| lsp.attach(&model, cx)).ok();
                }
                this.editor
                    .update(cx, |editor, cx| editor.set_model(model, cx));
                this.file_name = path.into();
                window.focus(&this.editor.focus_handle(cx), cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn cycle_language(&mut self, cx: &mut Context<Self>) {
        let model = self.editor.read(cx).model().clone();
        let current = model.read(cx).language_id().cloned();
        let ix = current
            .and_then(|id| self.languages.iter().position(|l| *l == id))
            .map_or(0, |ix| (ix + 1) % self.languages.len().max(1));
        if let Some(next) = self.languages.get(ix).cloned() {
            model.update(cx, |model, cx| model.set_language(Some(next), cx));
        }
        cx.notify();
    }

    fn cycle_theme(&mut self, cx: &mut Context<Self>) {
        self.theme = (self.theme + 1) % self.themes.len();
        let theme = self.themes[self.theme].1.clone();
        self.editor
            .update(cx, |editor, cx| editor.set_theme(theme, cx));
        cx.notify();
    }

    fn toggle(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut EditorOptions)) {
        self.editor.update(cx, |editor, cx| {
            let mut options = editor.options().clone();
            f(&mut options);
            editor.update_options(options, cx);
        });
        cx.notify();
    }
}

fn language_for_path(path: &str, cx: &App) -> SharedString {
    gpui_monaco::language_registry(cx)
        .language_for_path(path.as_ref())
        .map_or_else(|| "plaintext".into(), |l| l.id.to_string().into())
}

fn button(
    id: &'static str,
    label: impl Into<SharedString>,
    active: bool,
    colors: &EditorColors,
    on_click: impl Fn(&mut Playground, &mut Window, &mut Context<Playground>) + 'static,
    cx: &mut Context<Playground>,
) -> impl IntoElement {
    div()
        .id(id)
        .px_2()
        .py(px(2.))
        .rounded_sm()
        .cursor_pointer()
        .border_1()
        .border_color(if active {
            colors.input_option_active_border
        } else {
            colors.widget_border
        })
        .when(active, |this| {
            this.bg(colors.input_option_active_background)
        })
        .hover(|style| style.bg(gpui::hsla(0., 0., 0.5, 0.2)))
        .child(label.into())
        .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
}

impl Render for Playground {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let editor = self.editor.read(cx);
        let colors = editor.colors().clone();
        let options = editor.options().clone();
        let model = editor.model().read(cx);
        let language: SharedString = model
            .language_id()
            .cloned()
            .unwrap_or_else(|| "plaintext".into());
        let selections = editor.selections();
        let (line, column) = selections[0].head.to_monaco();
        let eol = match model.line_ending() {
            LineEnding::Lf => "LF",
            LineEnding::Crlf => "CRLF",
        };
        let mut status = vec![format!("Ln {line}, Col {column}")];
        if selections.len() > 1 {
            status.push(format!("{} selections", selections.len()));
        }
        status.extend([
            language.to_string(),
            eol.to_string(),
            self.file_name.to_string(),
        ]);
        let theme_name = self.themes.get(self.theme).map_or("custom", |(n, _)| *n);
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.background)
            .text_color(colors.widget_foreground)
            .text_size(px(12.))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_1()
                    .p_1()
                    .bg(colors.widget_background)
                    .border_b_1()
                    .border_color(colors.widget_border)
                    .child(button(
                        "open",
                        "Open…",
                        false,
                        &colors,
                        |this, window, cx| this.open_file(window, cx),
                        cx,
                    ))
                    .child(button(
                        "language",
                        format!("Language: {language}"),
                        false,
                        &colors,
                        |this, _, cx| this.cycle_language(cx),
                        cx,
                    ))
                    .child(button(
                        "theme",
                        format!("Theme: {theme_name}"),
                        false,
                        &colors,
                        |this, _, cx| this.cycle_theme(cx),
                        cx,
                    ))
                    .child(button(
                        "wrap",
                        "Wrap",
                        options.word_wrap != WordWrap::Off,
                        &colors,
                        |this, _, cx| {
                            this.toggle(cx, |o| {
                                o.word_wrap = if o.word_wrap == WordWrap::Off {
                                    WordWrap::On
                                } else {
                                    WordWrap::Off
                                }
                            })
                        },
                        cx,
                    ))
                    .child(button(
                        "minimap",
                        "Minimap",
                        options.minimap.enabled,
                        &colors,
                        |this, _, cx| this.toggle(cx, |o| o.minimap.enabled = !o.minimap.enabled),
                        cx,
                    ))
                    .child(button(
                        "sticky",
                        "Sticky scroll",
                        options.sticky_scroll.enabled,
                        &colors,
                        |this, _, cx| {
                            this.toggle(cx, |o| o.sticky_scroll.enabled = !o.sticky_scroll.enabled)
                        },
                        cx,
                    ))
                    .child(button(
                        "numbers",
                        "Line numbers",
                        options.line_numbers != LineNumbers::Off,
                        &colors,
                        |this, _, cx| {
                            this.toggle(cx, |o| {
                                o.line_numbers = if o.line_numbers == LineNumbers::Off {
                                    LineNumbers::On
                                } else {
                                    LineNumbers::Off
                                }
                            })
                        },
                        cx,
                    ))
                    .child(button(
                        "folding",
                        "Folding",
                        options.folding,
                        &colors,
                        |this, _, cx| this.toggle(cx, |o| o.folding = !o.folding),
                        cx,
                    ))
                    .child(button(
                        "readonly",
                        "Read-only",
                        options.read_only,
                        &colors,
                        |this, _, cx| this.toggle(cx, |o| o.read_only = !o.read_only),
                        cx,
                    ))
                    .child(button(
                        "whitespace",
                        "Whitespace",
                        options.render_whitespace == RenderWhitespace::All,
                        &colors,
                        |this, _, cx| {
                            this.toggle(cx, |o| {
                                o.render_whitespace =
                                    if o.render_whitespace == RenderWhitespace::All {
                                        RenderWhitespace::Selection
                                    } else {
                                        RenderWhitespace::All
                                    }
                            })
                        },
                        cx,
                    ))
                    .child(button(
                        "demo",
                        "Add demo widgets",
                        false,
                        &colors,
                        |this, _, cx| {
                            let editor = this.editor.clone();
                            add_demo_decorations(&editor, cx);
                            add_demo_widgets(&editor, cx);
                        },
                        cx,
                    )),
            )
            .child(div().flex_1().min_h_0().child(self.editor.clone()))
            .child(
                div()
                    .px_2()
                    .py(px(2.))
                    .bg(colors.input_option_active_border)
                    .text_color(gpui::white())
                    .flex()
                    .gap_4()
                    .children(status),
            )
    }
}

fn main() {
    let started = std::time::Instant::now();
    gpui_platform::application().run(move |cx: &mut App| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
        let bounds = Bounds::centered(None, size(px(1100.), px(760.)), cx);
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
        perf::run_if_requested(started, window, cx);
    });
}

/// A small label view for demo widgets.
struct DemoLabel {
    text: SharedString,
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
