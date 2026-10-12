//! gpui-monaco in the browser: an editor in a canvas through gpui's web
//! platform (`gpui-pre-web`). See `www/` and README.md for building it.
//!
//! It opens the playground's sample (`examples/playground/samples/sample.rs`).
//! The web platform ships no fonts, so the page passes one in: `run(font,
//! family)` registers the font's bytes and uses `family` for the editor.

#[cfg(target_family = "wasm")]
mod web {
    use std::borrow::Cow;
    use std::cell::RefCell;

    use gpui::{App, AppContext as _, ApplicationHandle, WindowOptions};
    use gpui_monaco::{CodeEditor, TextModel};
    use wasm_bindgen::prelude::*;

    /// The playground's sample, so the browser shows what the desktop demo does.
    const SAMPLE: &str = include_str!("../../playground/samples/sample.rs");

    thread_local! {
        /// Keeps the application alive after `run` returns.
        static APPLICATION: RefCell<Option<ApplicationHandle>> = const { RefCell::new(None) };
    }

    /// Starts the editor in a full-page canvas, with the font `font`
    /// (TrueType or OpenType bytes) whose family name is `family`.
    #[wasm_bindgen]
    pub fn run(font: Vec<u8>, family: String) -> Result<(), JsValue> {
        console_error_panic_hook::set_once();
        gpui_platform::web_init();
        let handle = gpui_platform::single_threaded_web().run_embedded(move |cx: &mut App| {
            cx.text_system()
                .add_fonts(vec![Cow::Owned(font)])
                .expect("failed to load the editor font");
            gpui_monaco::init(cx);
            gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
            cx.open_window(WindowOptions::default(), move |window, cx| {
                let model = cx.new(|_| TextModel::new(SAMPLE).with_language("rust"));
                cx.new(|cx| {
                    let mut editor = CodeEditor::new(model, window, cx);
                    let mut options = editor.options().clone();
                    options.font_family = family.into();
                    editor.update_options(options, cx);
                    editor
                })
            })
            .expect("failed to open the editor window");
            cx.activate(true);
        });
        APPLICATION.with(|application| *application.borrow_mut() = Some(handle));
        Ok(())
    }
}
