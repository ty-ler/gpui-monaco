# Editing features

Every feature here works out of the box. Each follows Monaco's behavior, which the ported Monaco tests check.

## Cursors and selection

- **Movement:** character, word, word part (camelCase and snake_case), line, page and document moves. Each has a selecting variant.
- **Keeping the column:** moving up or down remembers the column you started from. Tabs count as visible columns.
- **Paging:** PageUp and PageDown move the cursors by two lines less than the editor shows (Monaco's `pageSize`) and then scroll only as far as needed to show them. Cmd+PageUp/PageDown (Alt on Linux and Windows) scroll by the same amount, so two lines of the last page stay in view.
- **Revealing:** after a keyboard move the view shows every cursor, unless they're further apart than the editor is tall. Adding a cursor above or below, and column selection, follow the new cursor; Cmd+D centers the match it adds when it's off screen. The editor's API doesn't scroll: `set_position` and `set_selections` leave the view where it is, as Monaco's do.
- **Multiple cursors:**
  - Alt+Click adds a cursor.
  - Cmd+Alt+Up/Down adds a cursor on the line above or below (Shift+Alt+Up/Down on Linux).
  - Cmd+D adds the next match to the selection, Cmd+K Cmd+D moves the last selection to the next match, and Cmd+Shift+L selects all occurrences. Selected text matches with the find state's match case and whole word, both off unless set in the find widget (`selected_text_match_mode`); starting from an empty cursor matches whole words, case-sensitively.
  - Adding cursors and column selection are undo stops: undo after typing with the new cursors removes only that typing.
  - Escape returns to one cursor.
- **Column (box) selection:** Shift+Alt+drag or Shift+Alt+click (Shift+Cmd/Ctrl, or an Alt drag, with `multi_cursor_modifier: CtrlCmd`), a middle-button drag, or any drag with `column_selection`. It goes by lines as shown: a fold counts as one line, and each segment of a wrapped line gets a cursor.
- **Mouse:**
  - Double-click selects a word, triple-click a line and quadruple-click everything. Clicks count only at the same position within 400ms.
  - Dragging extends the selection by character, word or line, depending on the click count. Shift+click, Shift+double-click and Shift+triple-click extend the selection the same way.
  - Clicking the line numbers selects the line; Shift+click and dragging there extend by lines.
  - Alt+click adds a cursor, or removes the one clicked; Alt+double-click and Alt+triple-click select the new cursor's word or line.
  - Dragging past the editor's edge scrolls, faster the farther away the mouse is.
  - Cmd/Ctrl+click on a link or a symbol acts when the button comes up on the same line, so Cmd/Ctrl+drag selects as a plain drag does (see [links](providers.md#links) and [go to definition](providers.md#go-to-definition-references-and-the-peek-view)).
- **Selection anchor:** Cmd+K Cmd+B marks the cursor's position with a thin blue bar (hover it for "Selection Anchor"), Cmd+K Cmd+K selects from there to the cursor, and Escape removes it.
- **Clipboard:** copy, cut and paste with Monaco's multi-cursor rules. With one cursor per line, each cursor pastes its own line. With no selection, the whole line is copied (`empty_selection_clipboard`). Copies use the model's line ending (CRLF within the copied text on Windows, as Monaco). A column selection's copy pasted at a single cursor goes in as a block, with `multi_cursor_paste: Spread` (the default): each row on the next line at the cursor's visible column, short lines padded with spaces and rows past the end appended.
- **IME:** marked text is shown inline, underlined, with the clause being converted underlined thicker, and the candidate window is positioned at the cursor. With several cursors, the composition is typed at each of them. A committed character gets the typing helpers below, so a dead-key quote auto-closes or surrounds the selection.

## Typing helpers

- **Auto-closing:** typing an opening bracket or quote inserts the closing one. It follows the language's `autoClosingPairs`, `autoCloseBefore` and "not in strings or comments" rules.
- **Overtyping and deleting pairs:** typing a closing character you just auto-inserted moves past it. Backspace deletes both halves of the pair. Closing characters a snippet inserts count too, and so do pairs after undo or edits from another editor.
- **Auto-surround:** typing a bracket or quote with a selection wraps the selection.
- **Auto-indent:**
  - Enter applies the language's `onEnterRules` and `indentationRules`; for example, it continues `///` doc comments and indents after `{`.
  - Typing a closing bracket outdents the line.
  - Insert Line Above/Below indent the new line as Enter would, and indenting or outdenting lines keeps the ` * ` of doc comments aligned.
- **Comments:** Cmd+/ toggles line comments, Cmd+K Cmd+C adds them, Cmd+K Cmd+U removes them, and Shift+Alt+A toggles a block comment. The language at the cursor decides the tokens, so a Rust fence in Markdown gets `//` and Markdown text gets `<!-- -->`. Languages without a line comment fall back to block comments. `EditorOptions::comments` sets `insert_space` and `ignore_empty_lines`.
- **Line operations:**
  - Move, copy and delete lines, insert a line above or below, indent and outdent.
  - Join lines, sort, reverse and delete duplicate lines. Sorting orders lines as Monaco's default `Intl.Collator` does: punctuation, symbols, digits, then letters, with case and accents breaking ties (the Unicode root collation, whatever the locale).
  - Trim trailing whitespace, reindent, and insert a final new line ("Insert Final New Line"). Trimming leaves whitespace that starts in a string or regular expression, such as a template literal's, as standalone Monaco does.
- **Indentation:**
  - Models detect their indentation from their text when created (`detect_indentation`).
  - The commands "Convert Indentation to Spaces"/"to Tabs", "Indent Using Spaces"/"Tabs", "Change Tab Display Size" (a 1–8 picker that starts on the current size) and "Detect Indentation from Content" change it, as in Monaco.
- **Text transforms:**
  - Upper, lower, title, snake, camel, Pascal and kebab case.
  - Transpose characters, duplicate the selection and move the selected text left or right.
  - Step numbers and `true`/`false` with in-place replace: Cmd+Shift+, replaces with the previous value and Cmd+Shift+. with the next. In the `vs` theme the new value flashes an outline, as in Monaco.
  - Remove the brackets around the cursor (Cmd+Alt+Backspace).
  - Go to Bracket (Cmd+Shift+\\) moves to the matching bracket, the end of the enclosing pair, or the next bracket. Select to Bracket selects the pair, backwards from a closing bracket; `trigger_with_payload` with `{ "selectBrackets": false }` selects only what's between the brackets.

## Undo

Undo is operation-based and per model.

- **Grouping:** consecutive typing is grouped, and so are consecutive deletes. A cursor move or a different kind of edit starts a new group.
- **Cursors:** undo and redo restore the cursors of the editor that made the edit.
- **Explicit boundaries:** `push_stack_element` adds a boundary (Monaco `pushStackElement`).

## Find and replace

Cmd+F opens the find widget, and Cmd+Alt+F (Ctrl+H elsewhere) opens it with replace. The query starts as the selection or the word at the cursor (`options.find.seed_search_string_from_selection`), escaped as Monaco does when regex is on (`a.b` becomes `a\.b`, `a-b` stays). Selections can turn on find in selection (`auto_find_in_selection`). When the widget would cover the selection it was seeded from (one spanning lines, or reaching under the widget on the first lines), the text moves down below it instead of staying put. The widget slides down from above the editor as it opens, unless `App::reduce_motion` is set.

- **Search modes:**
  - case sensitive
  - whole word, using `word_separators`
  - regular expression
  - find in selection. As in Monaco, focusing the find or replace input takes the selections as the scope again, but a scope that's set only changes when one of the selections equals all of its ranges (so for a one-range scope, when the selections include it).
  - With find closed, Cmd+Alt+C, W, R and P (Alt+C, W, R and P elsewhere) still flip the toggles, and Monaco's small options widget shows the case, word and regex toggles at the top right for two seconds (longer while the pointer is over it).
- **Results:**
  - Every match is highlighted and marked in the overview ruler.
  - "N of M" counts up to 19,999 matches.
- **Navigation and replace:**
  - Enter and Shift+Enter move between matches.
  - In regex mode, Replace and Replace All expand `$1`, `$&` and `$$`, and `\u`, `\U`, `\l` and `\L` change the case of the next group (Monaco's replace patterns).
  - **Preserve case** (the `AB` toggle, Cmd+Alt+P / Alt+P) writes each replacement in the case of the text it replaces: `foo` → `bar`, `Foo` → `Bar`, `FOO` → `BAR`. Hyphenated and underscored words keep each part's case.
  - Navigation wraps at the first and last match unless `options.find.loop_` is off. Typing the query jumps to matches unless `options.find.cursor_move_on_type` is off. With `options.find.find_on_type` off, typing doesn't search at all until Enter.
  - "Go to Match..." (Monaco's `editor.action.goToMatchFindAction`) asks for a match number in the quick input; negative numbers count from the end.
  - `find_with_args` (Monaco's `editor.actions.findWithArgs`) opens find with a search string, replace string and toggles.
  - Alt+Enter in the find input selects all matches.
  - F3 / Cmd+G (Shift for previous) opens find if needed, seeded from the cursor, and moves to the next match. Cmd/Ctrl+F3 searches for the selection. Cmd+E (macOS) uses the selection, even over several lines.
  - Up and Down in the inputs walk the search and replace history.
  - Ctrl+Enter inserts a line break in the find and replace inputs (pasted line breaks stay too), so a search can span lines; the inputs grow with their lines, and Up and Down move between them before walking the history.
  - With `options.find.global_find_clipboard` on, find shares its search string through the system's find pasteboard on macOS, and with the app's other editors elsewhere: opening find with nothing to seed from the selection takes it, every new search string is written to it, and focusing the editor while find is open picks up one written meanwhile.
  - Dragging the widget's left edge widens it. Toggles carry over to new editors.

Regexes use the Rust `regex` crate. It runs in linear time but has no lookaround or backreferences; see [Differences from Monaco](differences.md).

## Folding

Fold ranges come from these sources, in order (Monaco's `foldingStrategy: auto`):

1. **Providers:** folding range providers (`register_folding_range_provider`), for example a language server's.
2. **Syntax tree:** the language's tree-sitter `folds.scm` query. Each `@fold` node folds from its first line to the line before its closing bracket or tag. Every bundled language has one. Region markers are added to these ranges.
3. **Indentation:** indentation plus `#region` / `#endregion` markers, where the language defines them.

`folding_strategy: Indentation` uses only the last source. Ranges are capped at `folding_maximum_regions` (5000): the most deeply nested provider and syntax ranges are dropped first, and the most indented indentation ranges, as in Monaco. `editor.folding_limit()` says how many were found and the limit applied, and `CodeEditorEvent::FoldingLimitChanged` reports changes, for a status bar warning. `api_commands::execute_folding_range_provider` computes a model's ranges without an editor. Recomputing waits for a 200ms pause in edits. Changing `folding_strategy`, the language or its configuration, and registering or unregistering a folding range provider (or `refresh_folding_ranges`) recompute at once; changing `folding` or `folding_maximum_regions` starts over, so turning folding off drops the folds. In the meantime the regions move with the text, so the chevrons and the folding commands stay on the right lines. Folded regions stay folded through edits, lines typed into them included, and keep their state when the new ranges arrive, unless their lines changed with a cursor in them. `editor.folding_ranges()` and `editor.collapsed_folds(cx)` read the current state.

- **Cursors:** 200ms after a cursor lands in a folded region, from find, Go to Line, Go to Definition, Jump to Bracket, Cmd+D or `set_selections`, the region unfolds and the cursor shows.

- **Gutter:** chevrons fade in on hover, or show always with `show_folding_controls: Always`; `Never` drops the column. A folded region ends with a grey "⋯"; click it to unfold.
  - Manual ranges have their own icons (Monaco's `folding-manual-collapsed` and `folding-manual-expanded`). They look like the others unless the host changes them with `view::folding::set_folding_icon(FoldingIcon::ManualCollapsed, Some(codicon), cx)`, as a VS Code product icon theme does.
  - A chevron toggles when the mouse is pressed and released on its line, as in Monaco; its first 4px are left to other gutter decorations.
  - Shift-click (or middle-click) a chevron to toggle the region with everything inside it.
  - Alt-click it to fold the regions around it instead (or unfold them if any are folded).
  - A click keeps the view on the clicked line rather than scrolling back to the cursor. The folding commands, Go to Parent/Previous/Next Fold included, show the cursor, centered if it was off screen.
  - A collapsed region the ranges no longer include stays folded, with the manual icons.
- **Placeholder:** a collapsed region shows `⋯` after its first line. Clicking it, or at the end of the line's text, unfolds the region and puts the cursor there, and so does clicking after the end of the line with `unfold_on_click_after_end_of_line`. With `folding_highlight` (the default), the line gets the `editor.foldBackground` color, in the minimap too.
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

Block comments and regions are found by kind when a folding range provider gives ranges with kinds. Otherwise they're found by the first line matching the language's block comment or region marker, as in Monaco. `folding_imports_by_default` collapses import ranges the first time a computation finds some, even from a provider registered later; a saved view state remembers it (`FoldingViewState::folded_imports`), so restored imports aren't folded again.
- **View state:** `save_view_state` keeps the collapsed and manual regions, and `restore_view_state` folds them again, leaving cursors where they were saved, even in a fold.

## Smart select

Expand Selection grows each selection through the enclosing syntax nodes: word part, word, call, statement, block and so on. Shrink Selection steps back. The keys are Ctrl+Cmd+Shift+Right/Left (or Ctrl+Shift+Right/Left) on macOS and Shift+Alt+Right/Left elsewhere; the methods are `expand_selection` and `shrink_selection`.

- **Range sources:** `SelectionRangeProvider`s (for example a language server's). Without one, the syntax tree's nodes are used, or bracket pairs when there's no grammar. Monaco's word ranges are always added: camelCase and snake_case parts, the word and the whole text.
- **Whole lines:** when a step crosses onto new lines, the lines without and with their indentation come in between (`smart_select.select_leading_and_trailing_whitespace`). `smart_select.select_subwords` turns off the word parts.
- **Reset:** moving the cursor starts over.

## Occurrences

- **Symbol at the cursor:** after the cursor rests on a word, its occurrences are highlighted.
  - They come from a [`DocumentHighlightProvider`](providers.md), or the word's whole-word occurrences.
  - F7 and Shift+F7 move the cursor to the start of the next and previous occurrence.
- **Selected text:** other occurrences of the selected text are highlighted. The highlight skips whitespace-only and multi-line selections, and selections longer than 200 characters.

## Quick access

Ctrl+G opens go to line, and Cmd/Ctrl+Shift+O opens go to symbol. Both use a quick input at the top of the editor, centered on it:

- **`:line` or `:line:column`:** goes to a line, previewing it as you type. Numbers are read as JavaScript's `parseInt` reads them (`:12.` is line 12), negative ones count from the end, and `::offset` goes to a character offset ("Go to Offset...", `editor.action.gotoOffset`, opens it with `::`). With `line_numbers: Relative`, the gutter shows absolute numbers while it's open.
- **`@name`:** goes to a symbol from the [document symbols](providers.md#document-symbols), with fuzzy matching. The whole query is tried against the name first, so names with spaces match; otherwise the first word matches the name and the others the container. `@:` groups the symbols by kind.
- **`>command`:** the command palette (F1). It runs any supported editor action by label, and recently used actions come first.
- **Anything else:** lists the prefixes.

Up and Down move through the list. Page Down focuses the last fully visible row, then scrolls a page; Page Up does the same upwards (Monaco's `focusNextPage` / `focusPreviousPage`). Enter goes to the item, while Escape (or clicking elsewhere) closes the list and restores the view. `CodeEditor::open_quick_access(prefix, …)` opens it from code.

Right-click or Shift+F10 opens the context menu, as does Ctrl+click on macOS (without selecting, whatever `multi_cursor_modifier` is). It holds the navigation, editing, clipboard and command palette items that can run. An item runs on mouse up, so a press can be dragged off it to cancel, and the wheel over the text closes the menu. Host actions join it with `.context_menu(group, order)`; Monaco's groups are `navigation`, `1_modification`, `9_cutcopypaste` and `z_commands`.

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
- Choices (`${1|one,two|}`), whose options show in the suggest widget. While a choice is active, Ctrl+Space shows its options again, and only them.
- Variables (`$TM_FILENAME`, `$CURRENT_YEAR`, `$CLIPBOARD`, …). The path variables use a `file:` URI's path, or another URI's path, as standalone Monaco does: `$TM_DIRECTORY` of `http://host/a/b` is `/a`.
- Transforms (`${1/(.*)/${1:/upcase}/}`).

Tab and Shift+Tab move between tab stops and Escape leaves the snippet. Snippet mode also ends at the last tab stop, when the cursor leaves the placeholders, or when the insertion is undone. Inserting a snippet inside an active one nests it into the current placeholder.

Lines are indented like the cursor's line unless `adjust_whitespace` is off. `overwrite_before` and `overwrite_after` count from the cursor, so with a selection the snippet replaces only those characters around its head. `set_snippet_workspace` sets what `$WORKSPACE_NAME` and `$WORKSPACE_FOLDER` refer to.

## Brackets

- **Highlighting:** while the editor has focus, the bracket pair next to the cursor is highlighted and marked in the overview ruler. With `match_brackets: Always` (the default, as in Monaco) the pair enclosing the cursor is highlighted when no bracket is next to it. Brackets inside strings and comments are skipped.
- **Navigation:** Cmd+Shift+\ jumps to the matching bracket.

## View

- **Word wrap:** follows Monaco's line-break rules, including CJK and break-before/break-after characters, plus `wrapping_indent`.
- **Drag and drop:** drag the selection to move it, or hold Alt (Ctrl on Windows and Linux) to copy it. Double-click just inside brackets or quotes to select their content.
- **Unicode highlighting:** ambiguous and invisible characters get a warning border. Hover one to learn why, or to stop highlighting it.
- **Minimap:** draws each character from Monaco's own minimap glyphs (or blocks with `render_characters` off), in its token color. The slider follows Monaco's `MinimapLayout`, so the end of a long file shows at the bottom and the slider follows the pointer when dragged. Clicking elsewhere centers the clicked line (`minimap.size: Proportional`) or moves the slider there (`Fill` and `Fit`). It shows the selections (`minimap.selectionHighlight`), find matches, occurrences, errors, warnings and infos, and decorations with `minimap` options. `// #region Name` and `// MARK: - Name` comments, and decorations with a section header, become section headers in it.
- **Sticky scroll:** pins up to `max_line_count` enclosing scope headers to the top, and at most a quarter of the editor's height. As a scope scrolls out, its header slides up under the one above. Clicking a header puts the cursor at the clicked column and reveals it, as Monaco's `revealPosition` does; Cmd/Ctrl+click goes to the definition of the word under the mouse.
  - The scopes come from `default_model`: the document symbols by default (Monaco's outline model), then the folding ranges or indentation. This doesn't need `folding` on.
  - Without symbols, it uses the folding ranges. The indentation model follows the language's `offSide` setting.
  - "Focus Editor Sticky Scroll" (command palette) puts the keyboard on the sticky lines. Use Up and Down, Enter to go to one, and Escape to return.
  - Shift over a sticky line shows the end of its scope; Shift+click goes there. The folding controls in the sticky gutter fold the scope without moving the cursor. With `show_folding_controls: Mouseover` the expanded ones fade in while the mouse is over the sticky line numbers, as the gutter's do; with `Never` there are none.
- **Scrollbars:** have an overview ruler showing the cursors, find matches, diagnostics and decorations. As in Monaco, a mark sits where its lines are in the whole scrollable height (view zones, padding and scrolling beyond the last line included), so it lines up with the slider, spans its lines (at least 6px), and marks of a color in the same lane that touch are merged. Once scrolled down, a 6px `scrollbar.shadow` shadow runs along the top (`scrollbar.use_shadows`).
