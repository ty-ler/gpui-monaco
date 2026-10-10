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

Fold ranges come from indentation, plus `#region` / `#endregion` markers where the language defines them.

- **Gutter:** chevrons show on hover, or always with `show_folding_controls: Always`.
- **Keys:**
  - Cmd+Alt+[ folds and Cmd+Alt+] unfolds.
  - Cmd+K Cmd+0 folds all and Cmd+K Cmd+J unfolds all.
- **Placeholder:** a collapsed region shows `⋯` after its first line. Unfold it with its gutter chevron or Cmd+Alt+].
- **Editing:** editing inside a collapsed region expands it, as in Monaco.

## Brackets

- **Highlighting:** the bracket pair around the cursor is highlighted (`match_brackets`). Brackets inside strings and comments are skipped.
- **Navigation:** Cmd+Shift+\ jumps to the matching bracket.

## View

- **Word wrap:** follows Monaco's line-break rules, including CJK and break-before/break-after characters, plus `wrapping_indent`.
- **Minimap:** shows colored blocks and a draggable slider.
- **Sticky scroll:** pins up to `max_line_count` enclosing scope headers to the top. Clicking a header jumps to it.
- **Scrollbars:** have an overview ruler showing the selection, find matches, diagnostics and decorations.
