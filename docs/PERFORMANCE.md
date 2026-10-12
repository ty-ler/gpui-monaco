# Performance

gpui-monaco targets large files: 100k+ lines and multi-megabyte files should stay smooth, and so should huge files and minified ones ([#59](https://github.com/ty-ler/gpui-monaco/issues/59)).

## Huge files and long lines

Measured in the playground's perf mode with `PLAYGROUND_PLAIN=1` (release, Apple M5 Pro):

| File | First frame | Keystroke, median / p95 | Page Down, median / p95 |
|---|---|---|---|
| 1M lines, 35MB of Rust | 340ms | 2.2 / 4.5ms | 2.0 / 4.3ms |
| One 2MB line of minified JS | 270ms | 11 / 16ms | 8.3 / 9.3ms |

What keeps them responsive:

- **Monaco's long-line limits:**
  - `stop_rendering_line_after` (10 000 by default) draws a "Show more" chip instead of the rest of the line.
  - `max_tokenization_line_length` (20 000, a `ModelOptions` field) leaves longer lines unhighlighted, so they count as plain text for brackets and indentation.
- **Chunked shaping:** long lines are shaped in chunks ending at delimiters, so an edit reshapes one chunk and GPUI's layout cache keeps the rest.
- **Windowed scans:** bracket matching only scans 20 000 units each way on a line. Cursor validation and the line queries cursor commands use slice the rope instead of copying whole lines.
- **One computation at a time:** fold ranges and bracket pairs run one whole-document pass at a time and catch up when it finishes, instead of piling up a pass per keystroke.
- **Batched lookups:** view zones (e.g. thousands of code lenses) find their rows in one forward pass over the text, and `Rope::offset_to_point` counts line breaks per chunk instead of walking characters.

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
  - The minimap walks highlights with a cursor and rebuilds in the background; after an edit it draws only the changed lines and keeps the rest of its tiles' pixels.
  - Bracket matching queries strings and comments a block of rows at a time.
  - Typing within a line updates only the edited rows' view-line counts.
  - `ViewLines` and `Buffer` are shared, not cloned, each frame.
