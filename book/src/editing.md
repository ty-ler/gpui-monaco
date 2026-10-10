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

Cmd+F opens the find widget, and Cmd+Alt+F (Ctrl+H elsewhere) opens it with replace. The query starts as the selection or the word at the cursor (`options.find.seed_search_string_from_selection`). Selections can turn on find in selection (`auto_find_in_selection`).

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
  - In regex mode, Replace and Replace All expand `$1`, `$&` and `$$`, and `\u`, `\U`, `\l` and `\L` change the case of the next group (Monaco's replace patterns).
  - **Preserve case** (the `AB` toggle, Cmd+Alt+P / Alt+P) writes each replacement in the case of the text it replaces: `foo` → `bar`, `Foo` → `Bar`, `FOO` → `BAR`. Hyphenated and underscored words keep each part's case.
  - Navigation wraps at the first and last match unless `options.find.loop_` is off. Typing the query jumps to matches unless `options.find.cursor_move_on_type` is off.
  - Alt+Enter in the find input selects all matches.
  - F3 / Cmd+G (Shift for previous) opens find if needed, seeded from the cursor, and moves to the next match. Cmd/Ctrl+F3 searches for the selection. Cmd+E (macOS) uses the selection, even over several lines.
  - Up and Down in the inputs walk the search and replace history.
  - Dragging the widget's left edge widens it. Toggles carry over to new editors.

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

## Smart select

Expand Selection grows each selection through the enclosing syntax nodes: word part, word, call, statement, block and so on. Shrink Selection steps back. The keys are Ctrl+Cmd+Shift+Right/Left (or Ctrl+Shift+Right/Left) on macOS and Shift+Alt+Right/Left elsewhere; the methods are `expand_selection` and `shrink_selection`.

- **Range sources:** `SelectionRangeProvider`s (for example a language server's). Without one, the syntax tree's nodes are used, or bracket pairs when there's no grammar. Monaco's word ranges are always added: camelCase and snake_case parts, the word and the whole text.
- **Whole lines:** when a step crosses onto new lines, the lines without and with their indentation come in between (`smart_select.select_leading_and_trailing_whitespace`). `smart_select.select_subwords` turns off the word parts.
- **Reset:** moving the cursor starts over.

## Occurrences

- **Symbol at the cursor:** after the cursor rests on a word, its occurrences are highlighted.
  - They come from a [`DocumentHighlightProvider`](providers.md), or the word's whole-word occurrences.
  - F7 and Shift+F7 select the next and previous occurrence.
- **Selected text:** other occurrences of the selected text are highlighted. The highlight skips whitespace-only and multi-line selections, and selections longer than 200 characters.

## Quick access

Ctrl+G opens go to line, and Cmd/Ctrl+Shift+O opens go to symbol. Both use a quick input at the top of the editor:

- **`:line` or `:line:column`:** goes to a line, previewing it as you type. Negative numbers count from the end, and `::offset` goes to a character offset.
- **`@name`:** goes to a symbol from the [document symbols](providers.md#document-symbols), with fuzzy matching. A second word matches the container. `@:` groups the symbols by kind.
- **`>command`:** the command palette (F1). It runs any supported editor action by label, and recently used actions come first.
- **Anything else:** lists the prefixes.

Up, Down, Page Up and Page Down move through the list. Enter goes to the item, while Escape (or clicking elsewhere) closes the list and restores the view. `CodeEditor::open_quick_access(prefix, …)` opens it from code.

Right-click or Shift+F10 opens the context menu. It holds the navigation, editing, clipboard and command palette items that can run. Host actions join it with `.context_menu(group, order)`; Monaco's groups are `navigation`, `1_modification`, `9_cutcopypaste` and `z_commands`.

Hosts add their own actions to the palette with Monaco's `addAction`:

```rust,ignore
editor.add_action(
    EditorActionDescriptor::new("my.format", "Format with Tool", |editor, window, cx| {
        // …
    })
    .alias("Prettify")
    .keybinding("cmd-k cmd-p")
    .precondition(|editor, _| !editor.options().read_only)
    .context_menu("1_modification", 1.5),
    cx,
);
editor.run_action("my.format", window, cx); // or dispatch RunAction { id }
```

## Snippets

`insert_snippet` inserts a TextMate snippet at every cursor and selects its first tab stop:

```rust,ignore
editor.update(cx, |editor, cx| {
    editor.insert_snippet(
        "for ${1:item} in ${2:iter} {\n\t$0\n}",
        SnippetInsertOptions::default(),
        cx,
    )
});
```

The supported syntax is Monaco's:
- Tab stops (`$1`, `$0`), placeholders (`${1:default}`) and mirrors (the same index used twice).
- Choices (`${1|one,two|}`), whose options show in the suggest widget.
- Variables (`$TM_FILENAME`, `$CURRENT_YEAR`, `$CLIPBOARD`, …).
- Transforms (`${1/(.*)/${1:/upcase}/}`).

Tab and Shift+Tab move between tab stops and Escape leaves the snippet. Snippet mode also ends at the last tab stop, when the cursor leaves the placeholders, or when the insertion is undone. Inserting a snippet inside an active one nests it into the current placeholder.

Lines are indented like the cursor's line unless `adjust_whitespace` is off. `set_snippet_workspace` sets what `$WORKSPACE_NAME` and `$WORKSPACE_FOLDER` refer to.

## Brackets

- **Highlighting:** the bracket pair around the cursor is highlighted (`match_brackets`). Brackets inside strings and comments are skipped.
- **Navigation:** Cmd+Shift+\ jumps to the matching bracket.

## View

- **Word wrap:** follows Monaco's line-break rules, including CJK and break-before/break-after characters, plus `wrapping_indent`.
- **Drag and drop:** drag the selection to move it, or hold Alt (Ctrl on Windows and Linux) to copy it. Double-click just inside brackets or quotes to select their content.
- **Unicode highlighting:** ambiguous and invisible characters get a warning border. Hover one to learn why, or to stop highlighting it.
- **Minimap:** shows colored blocks and a draggable slider. `// #region Name` and `// MARK: - Name` comments become section headers in it.
- **Sticky scroll:** pins up to `max_line_count` enclosing scope headers to the top. Clicking a header jumps to it.
  - The scopes come from `default_model`: the document symbols by default (Monaco's outline model), then the folding ranges or indentation.
  - Without symbols, it uses the folding ranges.
  - "Focus Editor Sticky Scroll" (command palette) puts the keyboard on the sticky lines. Use Up and Down, Enter to go to one, and Escape to return.
  - Shift over a sticky line shows the end of its scope; Shift+click goes there. The folding controls in the sticky gutter fold the scope.
- **Scrollbars:** have an overview ruler showing the selection, find matches, diagnostics and decorations.
