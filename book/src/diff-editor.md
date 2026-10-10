# Diff editor

`DiffEditor` shows two models side by side, like Monaco's `createDiffEditor`. Changed lines are tinted, changed characters are highlighted, and striped padding keeps the two sides aligned. Both sides scroll together.

With `render_side_by_side: false` the diff shows inline, like Monaco's inline diff. Only the modified editor is shown, and each change's deleted lines appear above it with syntax highlighting, the changed characters marked and `−` in the margin. Side-by-side diffs also switch to inline when the editor is narrower than `render_side_by_side_inline_breakpoint` (900px), unless `use_inline_view_when_space_is_limited` is turned off. `renders_side_by_side()` reports which mode is showing.

```rust
use gpui_monaco::{DiffEditor, TextModel};

let original = cx.new(|_| TextModel::new(&old_text).with_language("rust"));
let modified = cx.new(|_| TextModel::new(&new_text).with_language("rust"));
let diff = cx.new(|cx| DiffEditor::new(original, modified, window, cx));
```

- **Editors:** `original_editor()` and `modified_editor()` are ordinary `CodeEditor`s, so options, themes, decorations and providers work on each. The original side is read-only unless `DiffEditorOptions::original_editable` is set.
- **The diff:** `diff()` returns the latest `LinesDiff` (Monaco `getLineChanges`). It's recomputed in the background 200ms after an edit, and `DiffEditorEvent::DiffUpdated` fires when it changes.
- **Navigation:** `go_to_diff(next, window, cx)` moves to the next or previous change (Alt+F5 / Shift+Alt+F5).
- **Unchanged regions:** with `hide_unchanged_regions.enabled`, long runs of unchanged lines collapse into "N hidden lines" bars, leaving `context_line_count` lines (default 3) around each change. Click a bar's top or bottom edge to show `reveal_line_count` (default 20) more lines, or its `↕` button (or double-click the label) to show them all. Moving the cursor into hidden lines reveals them. `collapse_all_unchanged_regions` and `show_all_unchanged_regions` reset them.
- **Reverting:** a gutter between the sides (or left of the inline view) shows a "Revert Block" button beside the change at the cursor, and beside every change while the mouse is over it. With text selected, the button reverts only the selected changes. `revert(change, cx)`, `revert_range_mappings(changes, cx)` and the `Revert` action (the changes at the modified editor's cursor) do the same from code. The gutter is hidden when `render_gutter_menu` is off or the modified editor is read-only.
- **Accessible diff viewer:** F7 (or `accessible_diff_viewer_next`) opens a list of the changes for screen reader and keyboard users, like Monaco's. Each change shows with three lines of context and both line numbers. F7 and Shift+F7 move between changes, Up and Down between lines, Enter or Space jumps to the line in its editor, and Escape closes the list. `only_show_accessible_diff_viewer` shows only the list.
- **Moved blocks:** with `show_moves`, code that moved gets a border and a "Code moved to line …" caption on both sides, and a line joins the two places. The block at the cursor is highlighted.
- **Options:** `DiffEditorOptions` (Monaco `IDiffEditorOptions`):
  - `ignore_trim_whitespace`
  - `max_computation_time_ms`
  - `render_indicators` (`+`/`−` signs)
  - `render_overview_ruler`
  - `original_editable`
  - `enable_split_view_resizing` and `split_view_default_ratio` (drag the sash between the sides)
  - `show_moves` (detect and show moved blocks)
  - `hide_unchanged_regions` (`enabled`, `context_line_count`, `minimum_line_count`, `reveal_line_count`)
  - `render_gutter_menu`
  - `only_show_accessible_diff_viewer`
  - `render_side_by_side`, `use_inline_view_when_space_is_limited` and `render_side_by_side_inline_breakpoint`
- **Computing diffs directly:** `gpui_monaco::diff::compute_diff` is Monaco's diff algorithm on its own, for hosts that only need the changes.

Colors come from the theme: `diffEditor.insertedTextBackground`, `removedTextBackground`, `insertedLineBackground`, `removedLineBackground`, the `diffEditorGutter.*` backgrounds, `diffEditor.diagonalFill` and the `diffEditor.unchangedRegion*` and `unchangedCodeBackground` colors.
