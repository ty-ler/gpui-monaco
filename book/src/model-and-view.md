# Models and editors

## `TextModel`

A `TextModel` (Monaco `ITextModel`) holds:

- the text: a rope that normalizes line endings to `\n` and remembers the original line ending and BOM
- the language
- operation-based undo/redo
- decorations
- diagnostic markers

```rust,ignore
let model = cx.new(|_| {
    TextModel::new(&text)
        .with_language("rust")            // a language id from the registry
        .with_uri("file:///src/main.rs")  // passed to providers; optional
});
```

Edits use **byte offsets** into the current text and are applied together:

```rust,ignore
model.update(cx, |model, cx| {
    model.edit([(0..0, "// header\n"), (10..14, "")], cx).unwrap();
});
```

Other common calls:

| Call | Monaco equivalent |
|---|---|
| `model.text()` | `getValue()` |
| `model.set_value(text, cx)` (replaces the text and clears undo history) | `setValue` |
| `model.set_language(Some("json".into()), cx)` | `setModelLanguage` |
| `model.line_ending()` / `set_line_ending(eol, cx)` (emits `LineEndingChanged`) | `getEOL` / `setEOL` |
| `model.undo(None, cx)` / `redo(None, cx)` / `push_stack_element()` | `undo` / `redo` / `pushStackElement` |
| `model.snapshot()` | a cheap, immutable copy of the text for background work |

More `ITextModel` helpers:

- **Lines:** `line_count`, `line_content` and `line_len_utf16`.
- **Positions:** `validate_position` and `validate_range`, plus `offset_at` and `position_at` (byte offsets).
- **Text:** `value_in_range`.
- **Words:** `word_at_position` and `word_until_position`, using the language's `wordPattern`.
- **Search:** `find_matches`.

A model emits `TextModelEvent::Edited` for each change. Subscribe to it with `cx.subscribe(&model, …)` to sync with something else, such as an LSP server. The event carries:

- **`changes`:** Monaco's `IModelContentChange`s. Each has the replaced `range` (UTF-16 positions), its byte `range_offset` and `range_length`, and the new `text`. They're in descending order in the old text's coordinates, so applying them one after another reproduces the new text.
- **`patch`:** the same edits as byte ranges.
- **`kind`:** an edit, undo, redo or `set_value`.

### Coordinates

The text API uses three kinds of position:

- **Byte offsets** (`usize`), into the UTF-8 text.
- **`Point { row, column }`**: 0-based rows and byte columns.
- **`PointUtf16 { row, column }`**: 0-based rows and UTF-16 columns. This is the editor-facing type. Selections, providers and markers use it because it matches Monaco and LSP.

`PointUtf16::from_monaco(line, column)` and `to_monaco()` convert to and from Monaco's 1-based positions. The rope converts between all three kinds: `point_utf16_to_offset`, `offset_to_point_utf16`, and so on.

## `CodeEditor`

A `CodeEditor` (Monaco `ICodeEditor`) renders a model and edits it. It owns:

- cursors and selections
- scroll position
- options and theme
- folding state
- widgets

```rust,ignore
let editor = cx.new(|cx| CodeEditor::new(model.clone(), window, cx));
// Or, for a quick editor with its own model:
let editor = cx.new(|cx| CodeEditor::with_text("hello", window, cx));
```

Several editors can share one model, for example in a split view. An edit in one editor moves the others' cursors through the change, and undo restores the cursors of the editor that made the edit.

```rust,ignore
editor.update(cx, |editor, cx| {
    editor.set_selections(&[Selection::collapsed(PointUtf16::new(10, 0))], cx);
    editor.type_text("inserted", cx);       // like typing: auto-close, auto-indent, …
    let state = editor.save_view_state();   // selections and scroll
    editor.set_model(other_model, cx);
    editor.restore_view_state(&state, cx);
});
```

### Revealing, edits and commands

| Call | Monaco equivalent |
|---|---|
| `reveal_line`, `reveal_line_in_center`, `reveal_position`, `reveal_range(range, RevealType::…)` | `revealLine`, `revealLineInCenter`, `revealPosition`, `revealRange*` |
| `visible_ranges(cx)` | `getVisibleRanges` |
| `execute_edits(edits, Some(end_selections), cx)` | `executeEdits` |
| `push_undo_stop(cx)` | `pushUndoStop` |
| `trigger("code_editor::CommentLine", window, cx)` | `trigger` (with GPUI action names) |
| `has_text_focus(window)` | `hasTextFocus` |

### Events

`CodeEditor` emits `CodeEditorEvent`:

- `SelectionsChanged(SelectionChange)`: the `reason` is Monaco's `CursorChangeReason` (`Explicit`, `Paste`, `Undo`, `Redo`, `ContentFlush`, `RecoverFromMarkers` or `NotSet`). The `source` is `Keyboard`, `Mouse`, `Api`, `Model`, `ModelChange` or `RestoreState`.
- `ScrollChanged`
- `Focused`
- `Blurred`
- `MouseDown`, `MouseUp`, `MouseMove`, `MouseLeave` and `ContextMenu` (on right click): each carries an `EditorMouseEvent` whose `target` says what's under the mouse. The editor shows its own context menu unless the `context_menu` option is off.
  - Targets include the glyph margin, line numbers, line decorations, text, empty area, view zones, widgets, scrollbars, minimap and sticky scroll.
  - Each target also gives the text position. Use `editor.mouse_target(point, window, cx)` to hit-test any point.
  - The playground toggles breakpoints from glyph margin clicks.
- `OpenLocation(Location)`: a navigation command or the peek view wants a location in another model. Open it and call `go_to_location` on the new editor.

```rust,ignore
cx.subscribe(&editor, |this, editor, event: &CodeEditorEvent, cx| {
    if let CodeEditorEvent::SelectionsChanged(_) = event {
        let head = editor.read(cx).selections()[0].head;
        this.status = format!("Ln {}, Col {}", head.row + 1, head.column + 1);
        cx.notify();
    }
})
.detach();
```
