//! The large-file performance budget (`docs/PERFORMANCE.md`), on a ~100k-line
//! / ~3.5MB Rust file in a GPUI test window:
//!
//! - opening (model + editor + first frame) under 200ms,
//! - keystroke to painted frame under 8ms,
//! - reparsing after a keystroke off the main thread and under 50ms.
//!
//! The test platform shapes text with a no-op text system, so frame times
//! here are a lower bound; `PLAYGROUND_PERF=1` measures a real window.
//! `PERF_BUDGET_SCALE` multiplies the budgets for slow, noisy machines (CI
//! uses 2). Release builds only:
//!
//! ```sh
//! cargo test --release -p gpui-monaco --test perf_budget -- --ignored --nocapture
//! ```

use std::time::{Duration, Instant};

use gpui::{AppContext as _, Entity, EntityInputHandler, Focusable, TestAppContext};
use gpui_monaco::cursor::Selection;
use gpui_monaco::syntax::SyntaxSnapshot;
use gpui_monaco::text::{Bias, Buffer, PointUtf16};
use gpui_monaco::{CodeEditor, TextModel};

const OPEN_BUDGET: Duration = Duration::from_millis(200);
const KEYSTROKE_BUDGET: Duration = Duration::from_millis(8);
const REPARSE_BUDGET: Duration = Duration::from_millis(50);

fn budget(base: Duration) -> Duration {
    let scale = std::env::var("PERF_BUDGET_SCALE")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(1.);
    base.mul_f64(scale)
}

fn fixture() -> String {
    let sample = include_str!("../../../examples/playground/samples/sample.rs");
    sample.repeat(100_000 / sample.lines().count() + 1)
}

fn draw(cx: &mut gpui::VisualTestContext) {
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
}

fn p95(samples: &mut [Duration]) -> Duration {
    samples.sort();
    samples[(samples.len() - 1) * 95 / 100]
}

/// Budgets only mean something with optimizations on.
fn skip_debug() -> bool {
    if cfg!(debug_assertions) {
        eprintln!("perf budget skipped: run with --release");
    }
    cfg!(debug_assertions)
}

#[gpui::test]
#[ignore = "perf budget: run with --release -- --ignored"]
fn large_file_budget(cx: &mut TestAppContext) {
    if skip_debug() {
        return;
    }
    let text = fixture();
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
    });

    // Open into an existing window: the test context runs background tasks
    // (the parse) while creating one, which isn't part of opening a file.
    let (editor, cx) = cx.add_window_view(|window, cx| CodeEditor::with_text("", window, cx));
    draw(cx);
    let start = Instant::now();
    editor.update(cx, |editor, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_language("rust"));
        editor.set_model(model, cx);
    });
    draw(cx);
    let open = start.elapsed();
    println!("open: {open:?}");
    assert!(open < budget(OPEN_BUDGET), "open took {open:?}");

    // Parse in the background, then type in the middle of the file.
    cx.run_until_parked();
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    let middle = PointUtf16::new(50_000, 4);
    editor.update(cx, |editor, cx| {
        editor.set_selections(&[Selection::collapsed(middle)], cx)
    });
    draw(cx);
    let mut keystrokes = Vec::new();
    for _ in 0..100 {
        let start = Instant::now();
        type_char(&editor, cx);
        draw(cx);
        keystrokes.push(start.elapsed());
        // Background work (reparse, folding) runs between keystrokes.
        cx.run_until_parked();
    }
    let keystroke = p95(&mut keystrokes);
    println!("keystroke to paint p95: {keystroke:?}");
    assert!(
        keystroke < budget(KEYSTROKE_BUDGET),
        "keystroke p95 {keystroke:?}"
    );
}

fn type_char(editor: &Entity<CodeEditor>, cx: &mut gpui::VisualTestContext) {
    editor.update_in(cx, |editor, window, cx| {
        editor.replace_text_in_range(None, "x", window, cx)
    });
}

#[test]
#[ignore = "perf budget: run with --release -- --ignored"]
fn reparse_budget() {
    if skip_debug() {
        return;
    }
    let registry = gpui_monaco_languages::registry();
    let rust = registry.language_for_name("rust").unwrap();
    let mut buffer = Buffer::new(&fixture());
    let mut syntax = SyntaxSnapshot::parse(rust, &buffer.snapshot(), &registry);
    let mut samples = Vec::new();
    for i in 0..20 {
        let old = buffer.snapshot();
        let offset = buffer.clip_offset(buffer.len() / 20 * i, Bias::Left);
        buffer.edit([(offset..offset, "x")]).unwrap();
        let patch = buffer.edits_since(old.version());
        let start = Instant::now();
        syntax = syntax.reparse(old.rope(), &buffer.snapshot(), &patch, &registry);
        samples.push(start.elapsed());
    }
    let reparse = p95(&mut samples);
    println!("reparse p95: {reparse:?}");
    assert!(reparse < budget(REPARSE_BUDGET), "reparse p95 {reparse:?}");
}
