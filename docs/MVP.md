# gpui-monaco — MVP Definition

## Context
Build a Monaco-like code editor as a native GPUI component, published as a reusable public crate.
`/Users/tyler/projects/gpui-monaco` is empty (greenfield). This plan records the MVP scope decided in a
grilling session; every item below is a user decision unless marked *(derived)*.

## Repo & tracking
- **Repo:** https://github.com/ty-ler/gpui-monaco (private; `main` branch). Local checkout: `/Users/tyler/projects/gpui-monaco`, remote `origin`. This plan lives at `docs/MVP.md`.
- **Tracking issue:** [#61 MVP tracking](https://github.com/ty-ler/gpui-monaco/issues/61), a checklist of every issue grouped by milestone.
- **GitHub milestones** `M0 Skeleton` … `M8 Release`, plus `Post-MVP`. One issue per work item, each with Goal, Scope checklist, Acceptance and Depends-on links.
- **Labels:** `area:infra|text|syntax|editor|view|providers`, `conformance` (Monaco test ports), `perf`, `epic`, `post-mvp`.
- **Workflow:** one branch/PR per issue (`<issue#>-<slug>`), PR body `Closes #N`; tick scope checkboxes as they land. New work found mid-flight becomes a new issue linked from #61; scope changes update this doc and the issue in the same PR.

Key environment facts that shaped decisions:
- Zed's `rope`, `text`, `language`, `multi_buffer`, `editor` crates are **GPL-3.0** → not reusable. `gpui` and `sum_tree` are **Apache-2.0** → reusable.
- crates.io `gpui` is stale (0.2.2, Oct 2025); current gpui (upstream) has a split platform (`gpui_platform::application()`), unpublished platform crates.
- `gpui-universal` (user's, MIT/Apache) has `text_input.rs` / `input.rs` with a working `EntityInputHandler` + custom `Element` IME wiring — **reference only** (String storage, snapshot undo, full re-layout).
- Reference APIs in upstream gpui: `EntityInputHandler`/`ElementInputHandler` (`crates/gpui/src/input.rs`), `TextSystem::shape_line`, `LineWrapper`, `uniform_list`, `Element`, `actions!`/`KeyBinding`; example `crates/gpui/examples/input.rs`.

## Decisions

| Area | Decision |
|---|---|
| Audience | Public crate for any GPUI developer |
| Fidelity | **Behavior/UX-faithful** to Monaco; idiomatic Rust/GPUI API (not a 1:1 TS API clone) |
| License | MIT OR Apache-2.0; own text/editor stack |
| GPUI dep | The `gpui-pre` crates.io snapshots of zed (gpui, gpui_platform, sum_tree), pinned exactly to the snapshot gpui-component uses so apps can use both (#188; was: zed git at a pinned rev). Distribute via git until published to crates.io (#57) |
| Platforms | macOS first-class; Linux/Windows compile in CI, best-effort |
| Perf bar | Large files: 100k+ lines / multi-MB smooth. Huge/minified long lines = post-MVP |
| Text store | Rope on `sum_tree` (summaries: bytes, chars, UTF-16, lines); same sum_tree backs display-map layers |
| Undo | Operation-based undo/redo with edit grouping; **no CRDT** |
| Highlighting | Pluggable `Highlighter` trait; tree-sitter backend first (incremental, background parse). **Language injections required** *(derived: Markdown fences, HTML script/style)* |
| Languages | `LanguageRegistry` (grammar + highlights/indents/brackets/folds queries) + `gpui-monaco-languages` with cargo features: JS/TS/TSX, JSON/YAML/TOML, Rust, Markdown/HTML/CSS |
| Theming | Rust theme struct + VS Code/Monaco theme JSON loader (`colors` + `tokenColors`); tree-sitter captures → TextMate scopes |
| API shape | Model/View split: `Entity<TextModel>` (text, language, undo, decorations, markers) ⟷ `Entity<CodeEditor>` (cursors, scroll, view state; save/restore view state). Many editors per model |
| Keymap | Monaco default keymap per platform as GPUI actions in an editor key context; hosts rebind/unbind and register custom commands (`addAction`-like) |
| Lang intelligence | Provider traits only (host supplies data); LSP adapter crate post-MVP |
| Find regex | `regex` crate (linear-time); document differences from JS regex |
| A11y | Post-MVP; keep an accessible-text hook in view model; upstream ARIA patches from ty-ler/zed in parallel |
| Testing | Port Monaco's unit tests (cursor, editing, auto-close, indentation, find, word ops, text buffer) as a conformance oracle + GPUI `TestAppContext` tests + benches |

## MVP feature scope
**Editing:** typing, selection, clipboard, undo/redo, multi-cursor + column (box) select, find/replace widget (regex/case/whole-word, replace all), auto-close brackets/quotes, auto-indent, bracket matching, code folding.
**IDE widgets (provider-driven):** completion popup (filter/fuzzy/keyboard nav; snippets post-MVP), hover (markdown), diagnostics (squiggles, gutter + overview-ruler marks, `set_markers`), go-to-definition (Cmd+click/F12 hook), signature help.
**View:** gutter, line numbers, current-line highlight, scrollbars, word wrap, minimap, sticky scroll.
**Extensibility:** decorations API (`delta_decorations`: bg, underline, gutter glyph, inline before/after), content widgets, overlay widgets, view zones.
**Explicitly OUT:** diff editor, screen reader support, LSP client, snippets, code actions/rename/inlay hints, CRDT, wasm, crates.io publish, huge-line handling, Monaco migration notes.

## Proposed crate layout *(derived)*
- `crates/text` — rope, `Point`/offset/UTF-16 conversions, anchors, edits, undo history, EOL handling. **No gpui dep** (pure, fast tests).
- `crates/syntax` — `Highlighter` trait, tree-sitter backend, injections, `LanguageRegistry`, query types (highlights/indents/brackets/folds).
- `crates/gpui-monaco` — `TextModel`, `CodeEditor`, display map (folds → inline decorations → wrap → view zones/blocks), cursor/selection model, commands + keymap, editor `Element`, gutter, minimap, sticky scroll, find widget, provider traits + popups, widgets API, theme + VS Code theme loader.
- `crates/gpui-monaco-languages` — feature-flagged grammars and queries.
- `examples/playground` · `benches/` · `book/` (mdBook guide).

## Milestones
Issue numbers refer to https://github.com/ty-ler/gpui-monaco/issues.

- **M0 Skeleton:** workspace #1, pinned upstream gpui rev + hello window #2, CI #3, licenses/README #4.
- **M1 Text core:** rope #5, coordinates #6, edits #7, anchors #8, undo #9, EOL #10, Monaco text-buffer test port #11, rope benches #12.
- **M2 Basic editor:** `TextModel` #13, `CodeEditor` + view state #14, viewport-only Element #15, gutter #16, cursor/selection #17, mouse #18, IME #19, clipboard #20, scrolling #21, Monaco keymap #22, cursor/word-op test port #23.
- **M3 Syntax + theming:** `Highlighter`/`LanguageRegistry` #24, tree-sitter backend #25, injections #26, themes + VS Code loader #27, bracket matching #28, auto-close #29, auto-indent #30, bundled languages #31.
- **M4 Display map + extensibility:** display map #32, folding #33, word wrap #34, decorations #35, view zones #36, content/overlay widgets #37.
- **M5 Multi-cursor + find:** multi-cursor/box select #38, find/replace #39.
- **M6 Providers:** framework #40, completion #41, hover #42, diagnostics #43, go-to-def #44, signature help #45.
- **M7 Minimap + sticky scroll:** minimap #46, sticky scroll #47.
- **M8 Release:** playground #48, benches + perf budget #49, guide #50, v0.1 tag #51.
- **Post-MVP:** diff editor #52, a11y #53, LSP #54, snippets #55, code actions/rename/inlay hints #56, crates.io #57, wasm #58, huge files #59, migration notes #60.

## MVP deliverables
Playground example app (file open, language/theme switch, option toggles, mock providers); Criterion large-file benchmarks with CI perf budget; mdBook guide + rustdoc.

## Verification
- `cargo test --workspace`: ported Monaco conformance suites + `TestAppContext` editor tests pass.
- `cargo bench`: perf budget on a 100k-line / ~10MB file, e.g. open < 200ms, keystroke-to-paint < 8ms, scroll at 120fps on macOS, incremental reparse off the main thread.
- `cargo run -p playground`: manual check of every MVP feature on macOS, including IME (Japanese/Chinese input) and each bundled language.
- CI builds green on Linux and Windows.
