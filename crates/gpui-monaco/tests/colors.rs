//! Color decorators (Monaco `ColorDetector`): the built-in finder, color
//! providers, limits and options.

use std::time::Duration;

use gpui::{App, AppContext as _, Entity, Task, TestAppContext, VisualTestContext};
use gpui_monaco::color::ColorF;
use gpui_monaco::options::DefaultColorDecorators;
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, EditorOptions, TextModel};

const CSS: &str = "a {\n  color: #ff0000;\n  background: rgba(0, 0, 255, 0.5);\n}\n";

fn setup<'a>(
    text: &str,
    options: impl FnOnce(&mut EditorOptions),
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_language("css"));
        CodeEditor::new(model, window, cx)
    });
    editor.update(cx, |editor, cx| {
        let mut opts = editor.options().clone();
        options(&mut opts);
        editor.update_options(opts, cx);
    });
    (editor, cx)
}

fn settle(cx: &mut VisualTestContext) {
    for _ in 0..3 {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(1100));
        cx.run_until_parked();
    }
}

fn colors(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<(u32, u32, String)> {
    settle(cx);
    editor.read_with(cx, |e, cx| {
        e.colors_for_tests(cx)
            .into_iter()
            .map(|(range, hex)| (range.start.row, range.start.column, hex))
            .collect()
    })
}

fn swatches(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> usize {
    editor.read_with(cx, |e, _| e.color_swatch_count_for_tests())
}

/// Reports one green color at the start of the document.
struct GreenProvider;

impl DocumentColorProvider for GreenProvider {
    fn provide_document_colors(
        &self,
        _context: &ProviderContext,
        _cx: &mut App,
    ) -> Task<anyhow::Result<Vec<ColorInformation>>> {
        Task::ready(Ok(vec![ColorInformation {
            range: PointUtf16::new(0, 0)..PointUtf16::new(0, 1),
            color: ColorF {
                red: 0.,
                green: 1.,
                blue: 0.,
                alpha: 1.,
            },
        }]))
    }

    fn provide_color_presentations(
        &self,
        _context: &ProviderContext,
        _color: &ColorInformation,
        _cx: &mut App,
    ) -> Task<anyhow::Result<Vec<ColorPresentation>>> {
        Task::ready(Ok(Vec::new()))
    }
}

#[gpui::test]
fn finds_css_colors_without_providers(cx: &mut TestAppContext) {
    let (editor, cx) = setup(CSS, |_| {}, cx);
    assert_eq!(
        colors(&editor, cx),
        vec![
            (1, 9, "#ff0000ff".to_string()),
            (2, 14, "#0000ff80".to_string())
        ]
    );
    assert_eq!(swatches(&editor, cx), 2);

    // Edits move and update them.
    editor.update(cx, |e, cx| {
        e.model()
            .update(cx, |m, cx| m.set_value("b { color: #00ff00 }", cx))
    });
    assert_eq!(colors(&editor, cx), vec![(0, 11, "#00ff00ff".to_string())]);
}

#[gpui::test]
fn providers_replace_the_default_finder(cx: &mut TestAppContext) {
    let (editor, cx) = setup(CSS, |_| {}, cx);
    cx.update(|_, cx| {
        register_color_provider(cx, LanguageSelector::language("css"), GreenProvider);
    });
    assert_eq!(colors(&editor, cx), vec![(0, 0, "#00ff00ff".to_string())]);

    // `always` adds the default finder's colors.
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.default_color_decorators = DefaultColorDecorators::Always;
        e.update_options(options, cx);
    });
    assert_eq!(colors(&editor, cx).len(), 3);
}

#[gpui::test]
fn options_limit_and_disable(cx: &mut TestAppContext) {
    let (editor, cx) = setup(CSS, |o| o.color_decorators_limit = 1, cx);
    assert_eq!(colors(&editor, cx).len(), 2);
    assert_eq!(swatches(&editor, cx), 1);

    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.default_color_decorators = DefaultColorDecorators::Never;
        e.update_options(options, cx);
    });
    assert!(colors(&editor, cx).is_empty());
    assert_eq!(swatches(&editor, cx), 0);

    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.default_color_decorators = DefaultColorDecorators::Auto;
        options.color_decorators = false;
        e.update_options(options, cx);
    });
    assert!(colors(&editor, cx).is_empty());
}
