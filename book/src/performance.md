# Performance

gpui-monaco is built for large files. On an Apple M5 Pro, in a release build, with a 100k-line (3.5MB) Rust file:

| | Budget | Measured |
|---|---|---|
| Open the file in a running app | < 200ms | 7.5ms |
| Keystroke to painted frame, p95 | < 8ms | 5.8ms |
| Reparse after a keystroke | background, < 50ms | 10ms |
| Page Down to painted frame, p95 | about one frame | 8.5ms |

What makes this possible:

- **Storage:** text lives in a rope on `sum_tree`.
- **Layout and painting:** only the viewport is laid out and painted.
- **Background work:** these run on GPUI's background executor, with results arriving a frame later:
  - tree-sitter parsing
  - fold-range computation
  - find
  - minimap rendering

## Tips for hosts

- **Decorations:** keep decorations proportional to what's useful. Thousands are fine. Each frame queries only the visible range, but every decoration's range is updated on each edit.
- **Providers:** return `Task`s that do their real work off the main thread, for example with `cx.background_spawn`.
- **Snapshots:** use `model.snapshot()`, which is cheap to clone, when you need the text in a background task.

## Measuring

The repository's [`docs/PERFORMANCE.md`](https://github.com/ty-ler/gpui-monaco/blob/main/docs/PERFORMANCE.md) has full numbers and explains how to reproduce them:

- the component benchmarks (`cargo bench -p gpui-monaco --bench large_file`)
- the CI budget test (`tests/perf_budget.rs`)
- the playground's real-window perf mode (`PLAYGROUND_PERF=1`)
