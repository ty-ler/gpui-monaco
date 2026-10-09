//! `PLAYGROUND_PERF=1` (or `type` / `scroll` for just one part): measures
//! the editor in a real window and exits.
//!
//! Reports the time from startup to the first frame, then keystroke-to-paint
//! and Page Down-to-paint: the input is dispatched and a frame drawn
//! synchronously, so the numbers are main-thread CPU time (GPU presentation is
//! excluded). Background work (reparse, folding, find) runs between inputs as
//! it would while typing. Combine with `PLAYGROUND_FILE` for a large file; see
//! `docs/PERFORMANCE.md`.

use std::time::{Duration, Instant};

use gpui::{App, AppContext as _, Keystroke, WindowHandle};
use gpui_monaco::TextModel;

use crate::Playground;

pub fn run_if_requested(started: Instant, window: WindowHandle<Playground>, cx: &mut App) {
    let Ok(mode) = std::env::var("PLAYGROUND_PERF") else {
        return;
    };
    let typing_enabled = mode != "scroll";
    let scrolling_enabled = mode != "type";
    cx.spawn(async move |cx| {
        let draw = |cx: &mut gpui::AsyncApp| {
            cx.update_window(window.into(), |_, window, cx| {
                window.refresh();
                window.draw(cx).clear(cx);
            })
            .ok();
        };
        draw(cx);
        let open = started.elapsed();

        // Opening a file in a running app: a new model for the same text,
        // set on the editor and drawn.
        let reopen = cx
            .update_window(window.into(), |root, window, cx| {
                let playground = root.downcast::<Playground>().unwrap();
                let editor = playground.read(cx).editor.clone();
                let old = editor.read(cx).model().read(cx);
                let (text, language) = (old.text(), old.language_id().cloned());
                let start = Instant::now();
                let model = cx.new(|_| {
                    let model = TextModel::new(&text);
                    match language {
                        Some(language) => model.with_language(language),
                        None => model,
                    }
                });
                editor.update(cx, |editor, cx| editor.set_model(model, cx));
                window.refresh();
                window.draw(cx).clear(cx);
                start.elapsed()
            })
            .unwrap();
        // Let the background parse finish so keystrokes reparse incrementally.
        cx.background_executor()
            .timer(Duration::from_millis(1500))
            .await;

        let measure = |cx: &mut gpui::AsyncApp, keys: &[&str]| {
            let start = Instant::now();
            cx.update_window(window.into(), |_, window, cx| {
                for key in keys {
                    window.dispatch_keystroke(Keystroke::parse(key).unwrap(), cx);
                }
                window.draw(cx).clear(cx);
            })
            .ok();
            start.elapsed()
        };

        // Type in the middle of the file.
        measure(cx, &["cmd-down"]);
        for _ in 0..3 {
            measure(cx, &["pageup"]);
        }
        let mut typing = Vec::new();
        for i in 0..if typing_enabled { 200 } else { 0 } {
            let key = if i % 20 == 19 { "enter" } else { "x" };
            typing.push(measure(cx, &[key]));
            // ~60 keystrokes per second.
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
        }
        let mut scrolling = Vec::new();
        measure(cx, &["cmd-up"]);
        for _ in 0..if scrolling_enabled { 100 } else { 0 } {
            scrolling.push(measure(cx, &["pagedown"]));
            cx.background_executor()
                .timer(Duration::from_millis(if mode == "scroll" { 50 } else { 8 }))
                .await;
        }

        println!("startup to first frame: {:.1}ms", ms(open));
        println!("open file in running app: {:.1}ms", ms(reopen));
        if typing_enabled {
            report("keystroke to paint", &mut typing);
        }
        if scrolling_enabled {
            report("page down to paint", &mut scrolling);
        }
        cx.update(|cx| cx.quit());
    })
    .detach();
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.
}

fn report(name: &str, samples: &mut [Duration]) {
    samples.sort();
    let at = |q: f64| ms(samples[((samples.len() - 1) as f64 * q) as usize]);
    println!(
        "{name}: median {:.2}ms, p95 {:.2}ms, max {:.2}ms ({} samples)",
        at(0.5),
        at(0.95),
        at(1.),
        samples.len()
    );
}
