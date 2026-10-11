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
| `model.push_line_ending(eol, cx)` (an undo step, joining the open one like an edit) | `pushEOL` |
| `model.is_attached_to_editor()` / `attached_editor_count()`, `TextModelEvent::AttachedChanged` | `isAttachedToEditor` / `getAttachedEditorCount`, `onDidChangeAttached` |
| `model.undo(None, cx)` / `redo(None, cx)` / `push_stack_element()` | `undo` / `redo` / `pushStackElement` |
| `model.snapshot()` | a cheap, immutable copy of the text for background work |

More `ITextModel` helpers:

- **Lines:** `line_count`, `line_content`, `line_len_utf16` and `lines_content`.
- **Positions:** `validate_position` and `validate_range`, `offset_at` and `position_at` (byte offsets), `modify_position` (Monaco's UTF-16 offsets) and `full_model_range`.
- **Text:** `value_in_range` and `value_length`.
- **Words:** `word_at_position` and `word_until_position`, using the language's `wordPattern`.
- **Search:** `find_matches`, `find_matches_in` (with search scopes and regex captures), `find_next_match` and `find_previous_match`.
- **Edits:** `apply_edits` returns the edits that undo it. `push_edit_operations` joins the open undo step until `push_stack_element`, and takes a cursor state computer. `pop_stack_element` reopens the last step. `alternative_version_id` returns to earlier values on undo and redo, so it can tell whether the text is back to a saved state.
- **Brackets and guides:** `bracket_pairs_in_range`, `match_bracket`, `find_enclosing_brackets`, `lines_indent_guides` and `active_indent_guide`. Brackets in strings and comments don't count.
- **Indentation:** `normalize_indentation` (Monaco `normalizeIndentation`) rewrites a string's leading whitespace with the model's `indent_size` and `insert_spaces`. `line_first_non_whitespace_column` and `line_last_non_whitespace_column` are 0-based, `None` for blank lines (Monaco's `0`). `line_indent_column` counts leading spaces and tabs (`getLineIndentColumn`). `line_indent_level` is the indentation's width with tabs expanded, `None` for blank lines (Monaco `computeIndentLevel`).
- **Tokens:** `line_tokens(row, cx)` (Monaco `tokenization.getLineTokens`) gives a line's tokens like `colorize::tokenize`: each token's UTF-16 start, its Monaco token type with the language suffix (`keyword.rust`) and its language, the injected one inside a code fence. A model that hasn't parsed the latest text is tokenized right away.

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
    let state = editor.save_view_state(cx); // selections, scroll, folds
    editor.set_model(other_model, cx);
    editor.restore_view_state(&state, cx);
});
```

A `ViewState` (Monaco `ICodeEditorViewState`) holds the selections, the scroll position, the first visible position and the folded regions. Restoring it scrolls back to the same first line, even if view zones or wrapping moved it. A folded region comes back only if its lines haven't changed, which Monaco checks with a checksum.

### Revealing, edits and commands

| Call | Monaco equivalent |
|---|---|
| `reveal_line`, `reveal_line_in_center`, `reveal_lines…`, `reveal_position…`, `reveal_range(range, RevealType::…)`, `reveal_range_…` | `revealLine*`, `revealLines*`, `revealPosition*`, `revealRange*` |
| `reveal_range_with_scroll_type(range, RevealType::…, ScrollType::Immediate, cx)`, `reveal_lines_with_scroll_type` | the reveal methods' `scrollType` argument |
| `reveal_all_cursors(reveal_horizontal, minimal_reveal, cx)` | `revealAllCursors` |
| `visible_ranges(cx)` | `getVisibleRanges` |
| `execute_edits(edits, Some(end_selections), cx)` | `executeEdits` |
| `push_undo_stop(cx)` | `pushUndoStop` |
| `trigger("code_editor::CommentLine", window, cx)` | `trigger` (with GPUI action names) |
| `has_text_focus(window)`, `has_widget_focus(window, cx)` | `hasTextFocus`, `hasWidgetFocus` |

Reveals scroll smoothly (with `smooth_scrolling` on) unless given `ScrollType::Immediate`. A reveal that moves a line or less always jumps. `set_scroll_position`, `set_scroll_top` and `set_scroll_left` take a `ScrollType` (`set_scroll_position` jumps, like Monaco's default). `has_pending_scroll_animation` says whether a smooth scroll is still running.

### Geometry

These answer from the last frame, so they return `None` until the editor was painted. Pixel offsets are measured from the top of the content (or the start of the line), before scrolling.

| Call | Monaco equivalent |
|---|---|
| `layout_info()` → `LayoutInfo` | `getLayoutInfo` (`EditorLayoutInfo`: glyph margin, line numbers, decorations, content, minimap, scrollbars, overview ruler) |
| `top_for_line_number(row, include_view_zones)`, `bottom_for_line_number(row, …)`, `top_for_position(p)`, `line_height_for_position(p)` | `getTopForLineNumber`, `getBottomForLineNumber`, `getTopForPosition`, `getLineHeightForPosition` |
| `offset_for_column(row, column)`, `width_of_line(row)` | `getOffsetForColumn`, `getWidthOfLine` (rendered lines only) |
| `visible_column_from_position(p, cx)` | `getVisibleColumnFromPosition`, 0-based |
| `statusbar_column(p, cx)` | `getStatusbarColumn`: 1-based, tabs expanded, every other character one column |
| `content_width()`, `content_height()`, `scroll_width()`, `scroll_height()` | `getContentWidth`, `getContentHeight` (without `scroll_beyond_last_line`), `getScrollWidth`, `getScrollHeight` |
| `scroll_top()`, `scroll_left()` | `getScrollTop`, `getScrollLeft` (where a smooth scroll is going) |

### Events

`CodeEditor` emits `CodeEditorEvent`:

- `SelectionsChanged(SelectionChange)`: the `reason` is Monaco's `CursorChangeReason` (`Explicit`, `Paste`, `Undo`, `Redo`, `ContentFlush`, `RecoverFromMarkers` or `NotSet`). The `source` is `Keyboard`, `Mouse`, `Api`, `Model`, `ModelChange` or `RestoreState`.
- `ScrollChanged`
- `ReadOnlyEditAttempt`: an edit was refused because the editor is read-only. The editor also shows `read_only_message`.
- `Focused`
- `Blurred`
- `MouseDown`, `MouseUp`, `MouseMove`, `MouseLeave` and `ContextMenu` (on right click): each carries an `EditorMouseEvent` whose `target` says what's under the mouse. The editor shows its own context menu unless the `context_menu` option is off.
  - Targets include the glyph margin, line numbers, line decorations, text, empty area, view zones, widgets, scrollbars, minimap and sticky scroll.
  - Each target also gives the text position. Use `editor.mouse_target(point, window, cx)` to hit-test any point.
  - The playground toggles breakpoints from glyph margin clicks.
- `OpenLocation(Location)`: a navigation command or the peek view wants a location in another model. Open it and call `go_to_location` on the new editor.
- `WillChangeModel` and `ModelChanged`, with the old and new model's uri. `ModelLanguageChanged` forwards the model's language change.
- `ConfigurationChanged(change)`: some option changed. `change.has_changed(|o| &o.font_size)` says which.
- `LayoutChanged(LayoutInfo)` and `ContentSizeChanged`: sent after the frame that changed them, including the first frame.
- `WillType`/`DidType` (typed text), `WillPaste`/`DidPaste` (the pasted range and the language it was copied from), `WillCopy`/`WillCut`.
- `KeyDown` (sent before key bindings run) and `KeyUp`, `MouseWheel`, and `MouseDrag`/`MouseDrop`/`MouseDropCanceled` while the selection is dragged.
- `WidgetFocused`/`WidgetBlurred`: focus entered or left the editor, including its widgets such as the find input. `Focused`/`Blurred` track the text only.
- `CompositionStarted`/`CompositionEnded`: an input method marked text, then committed, cancelled or unmarked it. `in_composition()` says whether one is in progress.
- `HiddenAreasChanged`: folding or `set_hidden_areas` changed the hidden lines, reported when the next frame applies them.
- `ViewZonesChanged`: a view zone was added, removed or resized, including the editor's own (code lenses, peek views).
- `BeginUpdate`/`EndUpdate`: bracket an operation that may change the model or view several times (edits, typing, pasting, undo, redo, setting selections or the model), once however nested. As in Monaco, not every operation is bracketed.

These events are notifications. Unlike Monaco, a listener can't change the clipboard data or stop a key.

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

## Global registries

Monaco's `monaco.editor.getEditors`, `getDiffEditors` and `getModels` are `editors(cx)`, `diff_editors(cx)` and `models(cx)`. They hold weak handles, so released entities drop out. A diff editor's two code editors are listed too.

`on_did_create_editor`, `on_did_create_diff_editor`, `on_did_create_model`, `on_will_dispose_model` and `on_did_change_model_language` subscribe to the matching Monaco events and return a gpui `Subscription`:

```rust,ignore
gpui_monaco::on_did_create_model(cx, |model, cx| {
    println!("opened {:?}", model.read(cx).uri());
})
.detach();
gpui_monaco::on_will_dispose_model(cx, |_id, model, _cx| {
    println!("closed {:?}", model.uri());
})
.detach();
```

`init` starts tracking models, so call it before creating any. A model created earlier joins when an editor shows it.
