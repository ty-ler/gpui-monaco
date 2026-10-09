//! Background syntax parsing in `TextModel`.

use gpui::{AppContext as _, TestAppContext};
use gpui_monaco::TextModel;

#[gpui::test]
fn model_parses_in_background_and_after_edits(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry()));
    let model = cx.new(|_| TextModel::new("fn main() {}\n").with_language("rust"));
    model.update(cx, |model, cx| model.ensure_syntax(cx));
    cx.run_until_parked();
    let names = |model: &TextModel| {
        let syntax = model.syntax_snapshot().expect("parsed");
        syntax
            .highlighter
            .highlights(&syntax.state, syntax.text.rope(), 0..syntax.text.len())
            .iter()
            .map(|span| span.highlight.name().to_string())
            .collect::<Vec<_>>()
    };
    assert!(model.read_with(cx, |m, _| names(m)).contains(&"keyword".to_string()));

    model.update(cx, |model, cx| model.edit([(0..0, "// hi\n")], cx).unwrap());
    cx.run_until_parked();
    model.read_with(cx, |model, _| {
        let syntax = model.syntax_snapshot().unwrap();
        assert_eq!(syntax.text.version(), model.version());
        assert!(names(model).contains(&"comment".to_string()));
    });
}
