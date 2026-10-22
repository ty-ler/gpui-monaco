# Performance

gpui-monaco targets large files: 100k+ lines and multi-megabyte files should stay smooth, and so should huge files and minified ones ([#59](https://github.com/ty-ler/gpui-monaco/issues/59)).

## Huge files and long lines

Measured in the playground's perf mode with `PLAYGROUND_PLAIN=1` (release, Apple M5 Pro):

| File | First frame | Keystroke, median / p95 | Page Down, median / p95 |
|---|---|---|---|
| 1M lines, 35MB of Rust | 340ms | 2.2 / 4.5ms | 2.0 / 4.3ms |
| One 2MB line of minified JS | 270ms | 2.2 / 2.8ms | 8.3 / 9.3ms |

What keeps them responsive:

- **Monaco's long-line limits:**
  - `stop_rendering_line_after` (10 000 by default) draws a "Show more" chip instead of the rest of the line.
  - `max_tokenization_line_length` (20 000, a `ModelOptions` field) leaves longer lines unhighlighted, so they count as plain text for brackets and indentation.
  - Like Monaco (`canBuildAST`), a document longer than 5 000 000 UTF-16 units gets no bracket pairs: no bracket colors and no bracket pair guides, so no whole-document pass, and each shown line shapes as one run instead of being cut at every colored bracket.
- **Chunked shaping:** long lines are shaped in chunks of a few hundred bytes ending at delimiters, so an edit reshapes one chunk and GPUI's layout cache keeps the rest. The cuts are content-defined (a rolling hash of the bytes before a cut picks it, not the distance from the previous cut), so the chunks after an edit come back with the same text; with cuts counted from the previous one, a shifted cut moved every cut after it and a keystroke could reshape most of the line.
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

- **Playground perf mode:** `PLAYGROUND_PERF=type` or `PLAYGROUND_PERF=scroll` runs only one part. Typing happens 120 lines above the end of the file, with the cursor placed directly rather than by paging there, so paging changes don't move it. Each sample dispatches the input and then draws a frame synchronously, so the numbers are main-thread CPU time; GPU presentation is excluded.
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
| 277 lines | 1.1ms | 4.6ms | 3.6ms |
| 100k lines / 3.5MB | 5ms | 5.8ms | 8.5ms |
| 300k lines / 10MB | 11ms | 6.0ms | 9.5ms |

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
  - Bracket matching queries strings and comments a block of 16 rows at a time (small, since a parse drops the block an edit touches and the next bracket highlight queries it again), and the model keeps each block's, so cursor moves and keystrokes between parses reuse them (a block that an inserted or removed line break has shifted is kept by the range of the parsed text it maps to). A parse keeps the blocks it leaves alone: those no edit touches, on rows whose tokens it didn't change (tree-sitter's changed ranges), moved by the edits before them. Small files reparse after every keystroke, so otherwise each keystroke queried every block the search visits again. A bracket's offset on an ASCII row comes from the block's row starts, measured in one pass, instead of a rope seek. The scans read rows one after another with a `RowReader`, which goes on reading the rope's chunks from where the previous row ended (up or down) instead of seeking for each row; a long row is sliced with seeks as before, and the rows too long to tokenize are found in one pass over the short rows' text (a long row is measured with a seek and skipped, not read), not with seeks per row.
  - Typing updates only the edited rows' view-line counts, also when it adds or removes lines (unless it touches a fold).
  - `ViewLines` and `Buffer` are shared, not cloned, each frame.
  - View zones (thousands with code lenses) are laid out once and kept while the zones, their rows and sizes stay the same; only the zones on screen are visited to render them. After an edit, zone rows move by the lines the edits added or removed, and only zones in edited text are looked up again.
  - The syntax spans read for the visible lines are kept for their parse, so a frame showing the same lines of the same parse doesn't query the tree again.
  - Each shown row's text is read once per frame and shared by the passes that need it, only as far as its shown segments go (a long line stops rendering at 10 000 columns), and so is rendered whitespace's.
  - The syntax spans replaced by a newer parse's are dropped on the background executor: they can hold the last reference to the old tree, and freeing a big file's tree took several milliseconds of a frame.
  - Inlay hints for the scrolled-to rows replace only those rows' hints, as Monaco's `_updateHintsDecorators` does. Each hint keeps a slot with its decorations, so the hints elsewhere (all the hints shown so far) aren't moved, re-indexed or rehashed. While no edit has moved the decorations since they were shown, the rows' hints are found by binary search in the hints' order and spliced out. After an edit, every position is read from the decorations again.
  - `ViewLines` keeps its per-row data (projections and view-line counts) in `Arc`'d chunks of about 1,000 rows, with a prefix sum per chunk and the running total before each chunk. Views that keep an older mapping (the minimap's tiles plan their redraws from it) share the chunks, so an edit copies only the chunks it touches instead of the whole document, and dropping the old mapping frees only those. Inserting or deleting lines splices only the chunks at the edit's ends.
- **Decorations are found by id in constant time.** A model with more than 32 decorations indexes them by id when a lookup needs it, keeping the index while none are added, removed or reordered (edits move them in place), so per-marker lookups over thousands of markers (5000 are decorated, as in Monaco) don't scan every decoration each.
- **Per-keystroke work on a line scales with the columns it needs, not the line.** On a long line (minified code), fetching the whole line as a `String`, or encoding it to UTF-16, on every keystroke is most of the cost. `Rope::line_units_utf16` reads only the UTF-16 columns asked for, so ghost text and the suggest preview (`LineSource`), the minimap's decoration offsets (`ViewLines::line_content_prefix`, up to the minimap's width) and indent guides (leading whitespace only) read just that. `TextModel::word_at_position` (the suggest widget's word on every keystroke) reads only Monaco `getWordAtText`'s window of 500 units each side of the cursor on a line longer than 1000, and the suggest widget's indentation check and leading line content read only up to the cursor. Bracket matching skips the position-to-offset lookup when a block has no strings or comments, and checks a bracket's first code unit before comparing it.
- **Opening a file avoids per-line work.**
  - Indentation guessing streams its 10 000 lines from the rope instead of copying and encoding each one.
  - Rope chunks of ASCII text are measured a line at a time, not a char at a time.
  - Line endings and unusual line terminators are found by byte search.
  - `Rope::point_to_offset` and line lengths skip the char-by-char walk.
  - The first minimap tile reads from the rope only what fits in its width, not whole lines.
  - Code lens zones are removed in one pass.
  - The playground's demo providers scan the document with borrowed lines. Code lenses are found off the main thread. The inlay hints' document index is cached per model and version, so a scroll scans only the requested rows. Calls are found by looking up the identifier before each `(` in the index, not by searching each row for every function's name. The hints stay synchronous: hints that arrive a frame later make the next frame lay them out.
