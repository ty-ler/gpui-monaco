# Editing features

Every feature here works out of the box. Each follows Monaco's behavior, which the ported Monaco tests check.

## Cursors and selection

- **Movement:** character, word, word part (camelCase and snake_case), line, page and document moves. Each has a selecting variant.
- **Keeping the column:** moving up or down remembers the column you started from. Tabs count as visible columns.
- **Multiple cursors:**
  - Alt+Click adds a cursor.
  - Cmd+Alt+Up/Down adds a cursor on the line above or below (Shift+Alt+Up/Down on Linux).
  - Cmd+D adds the next match to the selection, Cmd+K Cmd+D moves the last selection to the next match, and Cmd+Shift+L selects all occurrences.
  - Escape returns to one cursor.
- **Column (box) selection:** Shift+Alt+drag.
- **Mouse:** double-click selects a word and triple-click selects a line. Dragging extends the selection by character, word or line, depending on the click count.
- **Clipboard:** copy, cut and paste with Monaco's multi-cursor rules. With one cursor per line, each cursor pastes its own line. With no selection, the whole line is copied (`empty_selection_clipboard`).
- **IME:** marked text is shown inline, and the candidate window is positioned at the cursor.

## Typing helpers

- **Auto-closing:** typing an opening bracket or quote inserts the closing one. It follows the language's `autoClosingPairs`, `autoCloseBefore` and "not in strings or comments" rules.
- **Overtyping and deleting pairs:** typing a closing character you just auto-inserted moves past it. Backspace deletes both halves of the pair.
- **Auto-surround:** typing a bracket or quote with a selection wraps the selection.
- **Auto-indent:**
  - Enter applies the language's `onEnterRules` and `indentationRules`; for example, it continues `///` doc comments and indents after `{`.
  - Typing a closing bracket outdents the line.
- **Comments:** Cmd+/ toggles line comments, Cmd+K Cmd+C adds them, Cmd+K Cmd+U removes them, and Shift+Alt+A toggles a block comment. The language at the cursor decides the tokens, so a Rust fence in Markdown gets `//` and Markdown text gets `<!-- -->`. Languages without a line comment fall back to block comments. `EditorOptions::comments` sets `insert_space` and `ignore_empty_lines`.
- **Line operations:**
  - Move, copy and delete lines, insert a line above or below, indent and outdent.
  - Join lines, sort, reverse and delete duplicate lines.
  - Trim trailing whitespace and reindent.
- **Text transforms:**
  - Upper, lower, title, snake, camel, Pascal and kebab case.
  - Transpose characters, duplicate the selection and move the selected text left or right.
  - Step numbers and `true`/`false` with in-place replace (Cmd+Shift+. / ,).
  - Remove the brackets around the cursor (Cmd+Alt+Backspace).

## Undo

Undo is operation-based and per model.

- **Grouping:** consecutive typing is grouped, and so are consecutive deletes. A cursor move or a different kind of edit starts a new group.
- **Cursors:** undo and redo restore the cursors of the editor that made the edit.
- **Explicit boundaries:** `push_stack_element` adds a boundary (Monaco `pushStackElement`).

## Find and replace

Cmd+F opens the find widget, and Cmd+Alt+F (Ctrl+H elsewhere) opens it with replace.

- **Search modes:**
  - case sensitive
  - whole word, using `word_separators`
  - regular expression
  - find in selection
- **Results:**
  - Every match is highlighted and marked in the overview ruler.
  - "N of M" counts up to 19,999 matches.
- **Navigation and replace:**
  - Enter and Shift+Enter move between matches.
  - Replace and Replace All support `$1` and `$&` substitution in regex mode.
  - Alt+Enter in the find input selects all matches.

Regexes use the Rust `regex` crate. It runs in linear time but has no lookaround or backreferences; see [Differences from Monaco](differences.md).

## Folding

Fold ranges come from these sources, in order (Monaco's `foldingStrategy: auto`):

1. **Providers:** folding range providers (`register_folding_range_provider`), for example a language server's.
2. **Syntax tree:** the language's tree-sitter `folds.scm` query. Each `@fold` node folds from its first line to the line before its closing bracket or tag. Every bundled language has one. Region markers are added to these ranges.
3. **Indentation:** indentation plus `#region` / `#endregion` markers, where the language defines them.

`folding_strategy: Indentation` uses only the last source. Provider and syntax ranges are capped at `folding_maximum_regions` (5000); the most deeply nested are dropped first, as in Monaco. Recomputing waits for a 200ms pause in edits. `editor.folding_ranges()` and `editor.collapsed_folds(cx)` read the current state.

- **Gutter:** chevrons show on hover, or always with `show_folding_controls: Always`.
  - Shift-click (or middle-click) a chevron to toggle the region with everything inside it.
  - Alt-click it to fold the regions around it instead (or unfold them if any are folded).
- **Placeholder:** a collapsed region shows `⋯` after its first line. Clicking it unfolds the region, and so does clicking after the end of the line with `unfold_on_click_after_end_of_line`. With `folding_highlight` (the default), the line gets the `editor.foldBackground` color.
- **Commands:** Monaco's, with Cmd on macOS and Ctrl elsewhere:

| Keys | Command |
|---|---|
| Cmd+Alt+[ / Cmd+Alt+] | Fold / Unfold (Ctrl+Shift+[ / ] elsewhere) |
| Cmd+K Cmd+L | Toggle Fold |
| Cmd+K Cmd+[ / Cmd+K Cmd+] | Fold / Unfold Recursively |
| Cmd+K Cmd+Shift+L | Toggle Fold Recursively |
| Cmd+K Cmd+0 / Cmd+K Cmd+J | Fold All / Unfold All |
| Cmd+K Cmd+1 … 7 | Fold Level 1 … 7, except regions around the cursors |
| Cmd+K Cmd+- / Cmd+K Cmd+= | Fold / Unfold All Except the regions around the cursors |
| Cmd+K Cmd+/ | Fold All Block Comments |
| Cmd+K Cmd+8 / Cmd+K Cmd+9 | Fold / Unfold All Regions (`#region` markers) |
| Cmd+K Cmd+, | Create a manual folding range from the selection (it's collapsed and kept across edits) |
| Cmd+K Cmd+. | Remove manual folding ranges at the cursor or in the selection |
| – | Go to Parent / Previous / Next Fold, Toggle Import Fold |

Block comments and regions are found by kind when a folding range provider gives kinds. Otherwise they're found by the first line matching the language's block comment or region marker, as in Monaco. `folding_imports_by_default` collapses import ranges when a model is first shown.
- **Editing:** editing inside a collapsed region expands it, as in Monaco.

## Occurrences

- **Symbol at the cursor:** after the cursor rests on a word, its occurrences are highlighted.
  - They come from a [`DocumentHighlightProvider`](providers.md), or the word's whole-word occurrences.
  - F7 and Shift+F7 select the next and previous occurrence.
- **Selected text:** other occurrences of the selected text are highlighted. The highlight skips whitespace-only and multi-line selections, and selections longer than 200 characters.

## Brackets

- **Highlighting:** the bracket pair around the cursor is highlighted (`match_brackets`). Brackets inside strings and comments are skipped.
- **Navigation:** Cmd+Shift+\ jumps to the matching bracket.

## View

- **Word wrap:** follows Monaco's line-break rules, including CJK and break-before/break-after characters, plus `wrapping_indent`.
- **Minimap:** shows colored blocks and a draggable slider.
- **Sticky scroll:** pins up to `max_line_count` enclosing scope headers to the top. Clicking a header jumps to it.
- **Scrollbars:** have an overview ruler showing the selection, find matches, diagnostics and decorations.
