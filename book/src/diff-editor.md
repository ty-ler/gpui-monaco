# Diff editor

`DiffEditor` shows two models side by side, like Monaco's `createDiffEditor`. Changed lines are tinted, changed characters are highlighted, and striped padding keeps the two sides aligned. Both sides scroll together.

```rust
use gpui_monaco::{DiffEditor, TextModel};

let original = cx.new(|_| TextModel::new(&old_text).with_language("rust"));
let modified = cx.new(|_| TextModel::new(&new_text).with_language("rust"));
let diff = cx.new(|cx| DiffEditor::new(original, modified, window, cx));
```

- **Editors:** `original_editor()` and `modified_editor()` are ordinary `CodeEditor`s, so options, themes, decorations and providers work on each. The original side is read-only unless `DiffEditorOptions::original_editable` is set.
- **The diff:** `diff()` returns the latest `LinesDiff` (Monaco `getLineChanges`). It's recomputed in the background 200ms after an edit, and `DiffEditorEvent::DiffUpdated` fires when it changes.
- **Navigation:** `go_to_diff(next, window, cx)` moves to the next or previous change (Alt+F5 / Shift+Alt+F5).
- **Options:** `DiffEditorOptions` (Monaco `IDiffEditorOptions`):
  - `ignore_trim_whitespace`
  - `max_computation_time_ms`
  - `render_indicators` (`+`/`−` signs)
  - `render_overview_ruler`
  - `original_editable`
  - `enable_split_view_resizing` and `split_view_default_ratio` (drag the sash between the sides)
  - `show_moves`
- **Computing diffs directly:** `gpui_monaco::diff::compute_diff` is Monaco's diff algorithm on its own, for hosts that only need the changes.

Colors come from the theme: `diffEditor.insertedTextBackground`, `removedTextBackground`, `insertedLineBackground`, `removedLineBackground`, the `diffEditorGutter.*` backgrounds and `diffEditor.diagonalFill`.
