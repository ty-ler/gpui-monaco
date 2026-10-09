# Performance

gpui-monaco targets large files: 100k+ lines and multi-megabyte files should stay smooth. Huge single lines, such as minified files, are post-MVP ([#59](https://github.com/ty-ler/gpui-monaco/issues/59)).

## Budget

| | Budget | Where it's enforced |
|---|---|---|
| Open a 100k-line file (model, editor, first frame) | < 200ms | `tests/perf_budget.rs`, playground perf mode |
| Keystroke to painted frame, p95 | < 8ms | `tests/perf_budget.rs`, playground perf mode |
| Reparse after a keystroke | off the main thread, < 50ms | `tests/perf_budget.rs` |
| Scroll (Page Down to painted frame), p95 | about one 120Hz frame | playground perf mode |

The main thread never parses. These all run on the background executor, and their results show up a frame or so later:

- tree-sitter parsing and reparsing
- fold ranges
- find matches
- the minimap

Until a parse lands, highlights from the previous tree are shifted through the newer edits.

## Measuring

```sh
# Component benchmarks: parse, reparse, highlights, wrap, folding, search
cargo bench -p gpui-monaco --bench large_file
cargo bench -p gpui-monaco-text            # rope / buffer

# The budget on the test platform (no real text shaping, so a lower bound)
cargo test --release -p gpui-monaco --test perf_budget -- --ignored --nocapture

# A real window: startup, open, keystroke-to-paint, page-down-to-paint
PLAYGROUND_PERF=1 PLAYGROUND_FILE=big.rs cargo run --release -p playground
```

- **Playground perf mode:** `PLAYGROUND_PERF=type` or `PLAYGROUND_PERF=scroll` runs only one part. Each sample dispatches the input and then draws a frame synchronously, so the numbers are main-thread CPU time; GPU presentation is excluded.
- **CI:** runs the budget test with `PERF_BUDGET_SCALE=2` to allow for shared runners, and runs the benchmarks with `--quick`.
- **Profiling on macOS:** build with `CARGO_PROFILE_RELEASE_DEBUG=line-tables-only`, run `dsymutil target/release/playground`, then use `sample <pid> 5` while the perf mode runs.

## Results

All results are from an Apple M5 Pro on release builds. The fixture is the playground sample repeated to about 100k lines (3.5MB of Rust, with about 5,400 macro injection layers).

### Benchmarks

| Benchmark | Time |
|---|---|
| syntax: full parse (background) | ~560ms |
| syntax: reparse after one keystroke (background) | 10ms |
| syntax: highlights for one screen | 0.14ms |
| view lines: build, no wrap | 0.3ms |
| view lines: wrap at 80 columns | 4.6ms |
| folding: indent ranges (background) | 2.8ms |
| search: all matches, literal or regex (background) | 4ms |
| text: load a 10MB buffer | 15.5ms |

### Real window

Measured with `PLAYGROUND_PERF=1`.

| File | Open in running app | Keystroke p95 | Page Down p95 |
|---|---|---|---|
| 277 lines | 0.8ms | 4.6ms | 3.6ms |
| 100k lines / 3.5MB | 7.5ms | 5.8ms | 8.5ms |
| 300k lines / 10MB | 26ms | 6.0ms | 9.5ms |

Startup to the first frame, including app and window creation, is about 200 to 300ms regardless of file size.

## Things that keep it fast

Each of these was a measured hot spot on large files:

- **Injection layers are reused.** A layer whose text didn't change keeps both its tree and its cached injection query results. The root layer re-runs its injection query only around changed ranges. Rust gets one injection layer per macro invocation, so this took reparse from 124ms to 10ms.
- **Layer index.** Highlighting looks up intersecting injection layers by position, so it doesn't scan all layers.
- **Whole-file passes stream lines.** Wrap, folding and find use `Rope::for_each_line` instead of a seek and allocation per row. Short lines skip the wrap algorithm. These changes took each pass from about 80ms to about 4ms.
- **Per-frame work scales with the viewport, not the file.**
  - Sticky scroll walks fold-range parents instead of scanning every range.
  - The fold gutter uses a binary search.
  - The minimap walks highlights with a cursor and rebuilds in the background.
  - Bracket matching queries strings and comments a block of rows at a time.
  - Typing within a line updates only the edited rows' view-line counts.
  - `ViewLines` and `Buffer` are shared, not cloned, each frame.
