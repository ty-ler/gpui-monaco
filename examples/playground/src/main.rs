mod perf;
mod providers;
mod screenshot;

use std::sync::Arc;

use gpui::{
    App, AppContext as _, Bounds, Context, Entity, Focusable, PathPromptOptions, SharedString,
    Window, WindowBounds, WindowOptions, div, prelude::*, px, size,
};
use gpui_monaco::options::{
    LineNumbers, RenderLineHighlight, RenderWhitespace, Ruler, ShowFoldingControls, WordWrap,
};
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
        ("hc-black", Arc::new(Theme::hc_black())),
        ("hc-light", Arc::new(Theme::hc_light())),
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
    /// `PLAYGROUND_EXTRA_FILE`: another file's model, so the demo
    /// references (Shift+F12) span two files.
    _extra_model: Option<Entity<TextModel>>,
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
        let extra_model = std::env::var("PLAYGROUND_EXTRA_FILE").ok().map(|path| {
            let text = std::fs::read_to_string(&path).unwrap_or_else(|e| format!("{path}: {e}"));
            let model = cx.new(|_| TextModel::new(&text).with_uri(file_uri(&path)));
            gpui_monaco::register_model(cx, &model);
            model
        });
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
        // Cmd/Ctrl+wheel zooms; `PLAYGROUND_FONT`, `PLAYGROUND_LIGATURES`
        // and `PLAYGROUND_ZOOM` pick the font, ligatures and zoom level.
        editor.update(cx, |editor, cx| {
            let mut options = editor.options().clone();
            options.mouse_wheel_zoom = true;
            if let Ok(family) = std::env::var("PLAYGROUND_FONT") {
                options.font_family = family.into();
            }
            if std::env::var("PLAYGROUND_WRAPPING").as_deref() == Ok("advanced") {
                options.wrapping_strategy = gpui_monaco::options::WrappingStrategy::Advanced;
            }
            if let Ok(placeholder) = std::env::var("PLAYGROUND_PLACEHOLDER") {
                options.placeholder = Some(placeholder.into());
            }
            if std::env::var_os("PLAYGROUND_READ_ONLY").is_some() {
                options.read_only = true;
            }
            if std::env::var_os("PLAYGROUND_LIGATURES").is_some() {
                options.font_ligatures = gpui_monaco::options::FontLigatures::On;
            }
            // `PLAYGROUND_LINE_HIGHLIGHT=none|gutter|line|all` and
            // `PLAYGROUND_FOLDING_CONTROLS=always|never|mouseover`.
            match std::env::var("PLAYGROUND_LINE_HIGHLIGHT").as_deref() {
                Ok("none") => options.render_line_highlight = RenderLineHighlight::None,
                Ok("gutter") => options.render_line_highlight = RenderLineHighlight::Gutter,
                Ok("line") => options.render_line_highlight = RenderLineHighlight::Line,
                Ok("all") => options.render_line_highlight = RenderLineHighlight::All,
                _ => {}
            }
            match std::env::var("PLAYGROUND_FOLDING_CONTROLS").as_deref() {
                Ok("always") => options.show_folding_controls = ShowFoldingControls::Always,
                Ok("never") => options.show_folding_controls = ShowFoldingControls::Never,
                Ok("mouseover") => options.show_folding_controls = ShowFoldingControls::Mouseover,
                _ => {}
            }
            // The glyph margin is on for the demo glyphs and breakpoints
            // (standalone Monaco's default is off);
            // `PLAYGROUND_NO_GLYPH_MARGIN=1` turns it off.
            options.glyph_margin = std::env::var_os("PLAYGROUND_NO_GLYPH_MARGIN").is_none();
            // `PLAYGROUND_INLAY_PADDING=1`: Monaco's `inlayHints.padding`.
            if std::env::var_os("PLAYGROUND_INLAY_PADDING").is_some() {
                options.inlay_hints.padding = true;
            }
            editor.update_options(options, cx);
        });
        if let Some(level) = std::env::var("PLAYGROUND_ZOOM")
            .ok()
            .and_then(|l| l.parse().ok())
        {
            gpui_monaco::set_editor_zoom_level(level, cx);
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
            editor.update(cx, |editor, cx| {
                let mut options = editor.options().clone();
                options.format_on_type = true;
                // Off by default in Monaco; on here to show it off in HTML.
                options.linked_editing = true;
                options.format_on_paste = true;
                // `PLAYGROUND_INLINE_TOOLBAR=always`: the inline suggestion
                // toolbar above the ghost text.
                if std::env::var("PLAYGROUND_INLINE_TOOLBAR").as_deref() == Ok("always") {
                    options.inline_suggest.show_toolbar =
                        gpui_monaco::options::InlineSuggestShowToolbar::Always;
                }
                editor.update_options(options, cx);
            });
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
            cx.subscribe(&editor, |_, _, event: &CodeEditorEvent, _| {
                if let CodeEditorEvent::SelectionsChanged(change) = event {
                    eprintln!(
                        "selections ({:?}, {:?}): {:?} {:?}",
                        change.reason, change.source, change.selection, change.secondary_selections
                    );
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
        // Links followed in any editor (Cmd/Ctrl+click), the ones in peek
        // views included (Monaco `registerLinkOpener`). Dropping the
        // `Disposable` keeps the opener.
        let _link_opener = gpui_monaco::register_link_opener(cx, |url, cx| {
            cx.open_url(url);
            true
        });
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
            _extra_model: extra_model,
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
        let start = gpui_monaco::text::PointUtf16::new(row, 0);
        let ids = model.update(cx, |model, cx| {
            model.delta_decorations_at(
                None,
                &[],
                vec![(
                    start..start,
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
        // Peek views can show files that aren't open.
        gpui_monaco::set_model_resolver(cx, |uri, cx| {
            let path = uri.strip_prefix("file://")?.replace("%20", " ");
            let text = std::fs::read_to_string(&path).ok()?;
            let language = language_for_path(&path, cx);
            Some(cx.new(|_| TextModel::new(&text).with_language(language).with_uri(uri)))
        });
        // `PLAYGROUND_WIDTH=400` makes a narrow window.
        let width = std::env::var("PLAYGROUND_WIDTH")
            .ok()
            .and_then(|w| w.parse::<f32>().ok())
            .unwrap_or(1100.);
        let bounds = Bounds::centered(None, size(px(width), px(760.)), cx);
        // `PLAYGROUND_MULTI_DIFF=old1:new1,old2:new2` opens a multi-diff editor
        // (an empty list shows its "No Changed Files" placeholder).
        // An empty side (`:new` or `old:`) is a created or deleted file, and
        // a file that isn't UTF-8 shows the binary placeholder.
        // `PLAYGROUND_MULTI_DIFF_VARIANT=cards|noCards|noCardsNonCompact`
        // picks the variant.
        if let Ok(pairs) = std::env::var("PLAYGROUND_MULTI_DIFF") {
            // The model, and whether the file is binary.
            let load = |path: &str, cx: &mut App| {
                if path.is_empty() {
                    return (None, false);
                }
                let (text, binary) = match std::fs::read_to_string(path) {
                    Ok(text) => (text, false),
                    Err(e) if e.kind() == std::io::ErrorKind::InvalidData => (String::new(), true),
                    Err(e) => (format!("{path}: {e}"), false),
                };
                let language = language_for_path(path, cx);
                let model = cx.new(|_| TextModel::new(&text).with_language(language));
                (Some(model), binary)
            };
            let documents: Vec<_> = pairs
                .split(',')
                .filter_map(|pair| pair.split_once(':'))
                .map(|(original, modified)| {
                    let path = if modified.is_empty() {
                        original
                    } else {
                        modified
                    };
                    let label = std::path::Path::new(path)
                        .file_name()
                        .map_or(path.to_string(), |n| n.to_string_lossy().into_owned());
                    let (original, original_binary) = load(original, cx);
                    let (modified, modified_binary) = load(modified, cx);
                    gpui_monaco::MultiDiffDocument {
                        label: label.into(),
                        original_label: None,
                        original,
                        modified,
                        binary: original_binary || modified_binary,
                        ..Default::default()
                    }
                })
                .collect();
            let variant = match std::env::var("PLAYGROUND_MULTI_DIFF_VARIANT").as_deref() {
                Ok("cards") => gpui_monaco::MultiDiffEditorVariant::Cards,
                Ok("noCards") => gpui_monaco::MultiDiffEditorVariant::NoCards,
                _ => gpui_monaco::MultiDiffEditorVariant::NoCardsNonCompact,
            };
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        ..Default::default()
                    },
                    |window, cx| {
                        cx.new(|cx| {
                            let mut multi =
                                gpui_monaco::MultiDiffEditor::new(documents, window, cx);
                            multi.set_variant(variant, cx);
                            // `PLAYGROUND_MULTI_DIFF_SCROLL=<px>` starts
                            // scrolled, e.g. to see a stuck header.
                            if let Some(top) = std::env::var("PLAYGROUND_MULTI_DIFF_SCROLL")
                                .ok()
                                .and_then(|top| top.parse::<f32>().ok())
                            {
                                multi.set_scroll_top(px(top), cx);
                            }
                            multi
                        })
                    },
                )
                .expect("failed to open window");
            cx.activate(true);
            screenshot::capture_if_requested(window.into(), cx);
            return;
        }
        // `PLAYGROUND_DIFF=original` compares it with `PLAYGROUND_FILE`
        // (`PLAYGROUND_DIFF_INLINE=1` for the inline view).
        if let (Ok(original), Ok(modified)) = (
            std::env::var("PLAYGROUND_DIFF"),
            std::env::var("PLAYGROUND_FILE"),
        ) {
            let load = |path: &str, cx: &mut App| {
                let text = std::fs::read_to_string(path).unwrap_or_else(|e| format!("{path}: {e}"));
                let language = language_for_path(path, cx);
                cx.new(|_| TextModel::new(&text).with_language(language))
            };
            let (original, modified) = (load(&original, cx), load(&modified, cx));
            // `PLAYGROUND_THEME=hc-black` (a built-in theme) themes both sides.
            if let Ok(name) = std::env::var("PLAYGROUND_THEME") {
                gpui_monaco::set_theme(cx, &name);
            }
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        ..Default::default()
                    },
                    |window, cx| {
                        let diff = cx.new(|cx| {
                            let mut diff =
                                gpui_monaco::DiffEditor::new(original, modified, window, cx);
                            // `PLAYGROUND_DIFF_INLINE=1` shows the inline view.
                            if std::env::var_os("PLAYGROUND_DIFF_INLINE")
                                .is_some_and(|v| !v.is_empty())
                            {
                                let mut options = diff.options().clone();
                                options.render_side_by_side = false;
                                diff.update_options(options, cx);
                            }
                            // `PLAYGROUND_DIFF_HIDE_UNCHANGED=1` collapses
                            // unchanged regions.
                            if std::env::var_os("PLAYGROUND_DIFF_HIDE_UNCHANGED")
                                .is_some_and(|v| !v.is_empty())
                            {
                                let mut options = diff.options().clone();
                                options.hide_unchanged_regions.enabled = true;
                                diff.update_options(options, cx);
                            }
                            // `PLAYGROUND_DIFF_MOVES=1` shows moved blocks.
                            if std::env::var_os("PLAYGROUND_DIFF_MOVES")
                                .is_some_and(|v| !v.is_empty())
                            {
                                let mut options = diff.options().clone();
                                options.show_moves = true;
                                diff.update_options(options, cx);
                            }
                            // `PLAYGROUND_DIFF_MARGIN_ARROWS=1` shows margin
                            // revert arrows instead of the gutter menu.
                            if std::env::var_os("PLAYGROUND_DIFF_MARGIN_ARROWS")
                                .is_some_and(|v| !v.is_empty())
                            {
                                let mut options = diff.options().clone();
                                options.render_gutter_menu = false;
                                diff.update_options(options, cx);
                            }
                            // `PLAYGROUND_DIFF_LEGACY=1` uses the legacy diff
                            // algorithm (no inner changes for pure deletions).
                            if std::env::var_os("PLAYGROUND_DIFF_LEGACY")
                                .is_some_and(|v| !v.is_empty())
                            {
                                let mut options = diff.options().clone();
                                options.diff_algorithm = gpui_monaco::DiffAlgorithm::Legacy;
                                diff.update_options(options, cx);
                            }
                            // `PLAYGROUND_DIFF_WHITESPACE=1` renders whitespace
                            // and control characters on both sides.
                            if std::env::var_os("PLAYGROUND_DIFF_WHITESPACE")
                                .is_some_and(|v| !v.is_empty())
                            {
                                for editor in [diff.original_editor(), diff.modified_editor()] {
                                    editor.update(cx, |editor, cx| {
                                        let mut options = editor.options().clone();
                                        options.render_whitespace = RenderWhitespace::All;
                                        options.render_control_characters = true;
                                        editor.update_options(options, cx);
                                    });
                                }
                            }
                            // `PLAYGROUND_DIFF_HUNK_ACTIONS=N` adds N host
                            // buttons to the gutter's hunk toolbars.
                            let hunk_actions = std::env::var("PLAYGROUND_DIFF_HUNK_ACTIONS")
                                .ok()
                                .and_then(|n| n.parse::<usize>().ok())
                                .unwrap_or(0);
                            for i in 1..=hunk_actions {
                                diff.add_hunk_action(
                                    gpui_monaco::DiffHunkAction {
                                        id: format!("playground.action{i}").into(),
                                        label: format!("Action {i}").into(),
                                        icon: "$(add)".into(),
                                        toolbar: gpui_monaco::DiffHunkToolbar::Hunk,
                                        run: std::rc::Rc::new(|_, _, _| {}),
                                    },
                                    cx,
                                );
                            }
                            diff
                        });
                        // Typing goes to the modified side.
                        window.focus(&diff.read(cx).focus_handle(cx), cx);
                        diff
                    },
                )
                .expect("failed to open window");
            cx.activate(true);
            screenshot::capture_if_requested(window.into(), cx);
            return;
        }
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

/// A glyph margin widget's view: a dot.
struct DemoGlyph;

impl Render for DemoGlyph {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(div().size(px(8.)).rounded_full().bg(gpui::rgb(0xe51400)))
    }
}

/// A view zone, a content widget, an overlay widget and a glyph margin
/// widget.
fn add_demo_widgets(editor: &Entity<CodeEditor>, cx: &mut App) {
    use gpui::{hsla, rgb};
    use gpui_monaco::{
        ContentWidget, ContentWidgetPreference, GlyphMarginWidget, OverlayPosition, OverlayWidget,
        ViewZone, ZoneHeight,
    };
    let zone = cx.new(|_| DemoLabel {
        text: "A view zone between lines 4 and 5 — any GPUI view".into(),
        background: hsla(0.6, 0.5, 0.3, 1.),
    });
    let margin = cx.new(|_| DemoLabel {
        text: "margin".into(),
        background: hsla(0.6, 0.5, 0.2, 1.),
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
        // With a margin part in the gutter (Monaco `marginDomNode`).
        let mut zone = ViewZone::new(4, ZoneHeight::Lines(2.), zone.into());
        zone.margin_view = Some(margin.into());
        editor.add_view_zone(zone, cx);
        editor.add_content_widget(
            ContentWidget::new(
                gpui_monaco::text::PointUtf16::new(16, 4),
                vec![
                    ContentWidgetPreference::Above,
                    ContentWidgetPreference::Below,
                ],
                content.into(),
            ),
            cx,
        );
        editor.add_overlay_widget(
            OverlayWidget::new(OverlayPosition::TopRight, overlay.into()),
            cx,
        );
        // For lines 21 to 30: beside line 21, or the first line in view
        // while it's scrolled past.
        editor.add_glyph_margin_widget(
            GlyphMarginWidget::with_range(
                gpui_monaco::text::PointUtf16::new(20, 0)
                    ..gpui_monaco::text::PointUtf16::new(29, 0),
                cx.new(|_| DemoGlyph),
            ),
            cx,
        );
    });
}

/// Decorations showing off each decoration style.
fn add_demo_decorations(editor: &Entity<CodeEditor>, cx: &mut App) {
    use gpui::{hsla, rgb};
    use gpui_monaco::{
        DecorationColor, DecorationOptions, DecorationUnderline, GlyphMarginLane,
        GlyphMarginOptions, InjectedText, OverviewRulerDecoration, OverviewRulerLane, TokenStyle,
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
                        overview_ruler: Some(OverviewRulerDecoration::new(
                            error,
                            OverviewRulerLane::Right,
                        )),
                        hover_message: vec!["demo error".into()],
                        glyph: Some(("●".into(), error)),
                        glyph_margin_hover_message: vec![
                            "**Breakpoint** on `transform_offset`".into(),
                        ],
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
                        // A theme color (Monaco `ThemeColor`), following theme
                        // changes.
                        overview_ruler: Some(OverviewRulerDecoration::new(
                            DecorationColor::theme("editorOverviewRuler.rangeHighlightForeground"),
                            OverviewRulerLane::Left,
                        )),
                        // Monaco `lineHeight`: a taller line.
                        line_height: Some(1.8),
                        // Whole-line hovers (a blame annotation) and line
                        // number hovers.
                        hover_message: vec!["*Blame:* added in the first commit".into()],
                        line_number_hover_message: vec!["Line changed".into()],
                        // As a debugger's line highlight: whole-line
                        // decorations stay under the others (find matches,
                        // word highlights) whatever their z-index.
                        z_index: 20,
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
                            swatch: None,
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                ),
                // Glyph margin lanes (Monaco `glyphMargin.position`): a
                // range in the left lane, a line in the right one.
                (
                    find("impl Edit {").start..find("fn delta").end,
                    DecorationOptions {
                        glyph: Some(("◆".into(), rgb(0x3794ff).into())),
                        glyph_margin: Some(GlyphMarginOptions {
                            position: GlyphMarginLane::Left,
                            persist_lane: false,
                        }),
                        glyph_margin_hover_message: vec![
                            "Left lane: `impl Edit`".into(),
                            gpui_monaco::providers::MarkdownString::new(
                                "$(info) on every line of the range",
                            )
                            .with_theme_icons(true),
                        ],
                        ..Default::default()
                    },
                ),
                (
                    find("pub fn new_len"),
                    DecorationOptions {
                        glyph: Some(("●".into(), warning)),
                        glyph_margin: Some(GlyphMarginOptions {
                            position: GlyphMarginLane::Right,
                            persist_lane: false,
                        }),
                        glyph_margin_hover_message: vec!["Right lane: `new_len`".into()],
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
