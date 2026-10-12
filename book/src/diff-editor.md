# Diff editor

`DiffEditor` shows two models side by side, like Monaco's `createDiffEditor`. Changed lines are tinted, changed characters are highlighted, and striped padding keeps the two sides aligned. Both sides scroll together.

With `render_side_by_side: false` the diff shows inline, like Monaco's inline diff. The modified editor shows each change's deleted lines above it with syntax highlighting, the changed characters marked and `−` in the margin; they wrap like the modified editor's lines, widen its horizontal scroll range, can be selected and copied, and a light bulb in their margin offers "Copy deleted lines", "Copy deleted line (N)" and "Revert this change". Left of it, the original editor shows as a column of its line numbers, unless `hide_original_line_numbers` (or `compact_mode`) is set. Side-by-side diffs also switch to inline when the editor is narrower than `render_side_by_side_inline_breakpoint` (900px), unless `use_inline_view_when_space_is_limited` is turned off or the modified editor runs optimized for screen readers (`accessibility_support`). `renders_side_by_side()` reports which mode is showing.

```rust
use gpui_monaco::{DiffEditor, TextModel};

let original = cx.new(|_| TextModel::new(&old_text).with_language("rust"));
let modified = cx.new(|_| TextModel::new(&new_text).with_language("rust"));
let diff = cx.new(|cx| DiffEditor::new(original, modified, window, cx));
```

- **Editors:** `original_editor()` and `modified_editor()` are ordinary `CodeEditor`s, so options, themes, decorations and providers work on each. The original side is read-only unless `DiffEditorOptions::original_editable` is set. As in Monaco, the diff editor turns off both editors' minimap and folding, their sticky scroll while unchanged regions are hidden, and, inline, the original editor's sticky scroll, unicode highlighting and glyph margin; it keeps these off if a host turns them back on. `add_action` and `add_command` add to the modified editor, like Monaco's `IStandaloneDiffEditor`.
- **Alignment:** padding keeps the sides aligned around every view zone (code lenses, peek views, the find and problem zones, host zones), wrapped lines and line height changes, and is recomputed when any of them change.
- **The diff:** `diff()` returns the latest `LinesDiff` (Monaco `getDiffComputationResult().changes2`), `line_changes()` the same as Monaco's `ILineChange`s (`getLineChanges`) and `identical()` whether the texts are equal. It's recomputed in the background 200ms after an edit (meanwhile the edit moves its line ranges), `DiffEditorEvent::DiffUpdated` fires when it changes, and `wait_for_diff()` resolves once it's computed.
- **Navigation:** `go_to_diff(next, window, cx)` moves to the next or previous change (Alt+F5 / Shift+Alt+F5). `reveal_first_diff(cx)` (Monaco `revealFirstDiff`) moves to the first change once the diff is computed. `switch_side(window, cx)` (`diffEditor.switchSide`, the `SwitchSide` action) focuses the other side with the selection mapped through the diff, and `map_to_other_side` returns that mapping.
- **Models and view state:** `set_models` (or `set_model(Some(DiffEditorModel { .. }))`) emits `DiffEditorEvent::ModelChanged` (Monaco `onDidChangeModel`); `set_model(None)` shows nothing, and `model()` returns the models. `save_view_state(cx)` returns a `DiffEditorViewState` with both editors' view states and the collapsed unchanged regions, and `restore_view_state` applies it (the regions once the diff is computed).
- **Commands:** `trigger` runs the `diffEditor.*` ids: `revert`, `switchSide`, `collapseAllUnchangedRegions`, `showAllUnchangedRegions`, `exitCompareMove`, `toggleCollapseUnchangedRegions`, `toggleShowMovedCodeBlocks` and `toggleUseInlineViewWhenSpaceIsLimited` (the last three are also GPUI actions, unbound like Monaco's).
- **Unchanged regions:** with `hide_unchanged_regions.enabled`, long runs of unchanged lines collapse into "N hidden lines" bars, leaving `context_line_count` lines (default 3) around each change. Click a bar's top or bottom edge to show `reveal_line_count` (default 20) more lines, or drag the edge to show as many as you like (the code below a bottom edge stays put), or its `↕` button (or double-click the label) to show them all. A fully shown region gets a "Fold Unchanged Region" glyph to hide it again. Bars also show the symbols the hidden lines start (from the modified model's document symbol providers, Monaco's `diffEditorBreadcrumbs`); clicking one reveals it. Moving the cursor into hidden lines reveals them. `collapse_all_unchanged_regions` and `show_all_unchanged_regions` reset them.
- **Reverting and the gutter menu:** a gutter between the sides (or left of the inline view) shows a toolbar beside the change at the cursor, and beside every change while the mouse is over it (they fade out after). Its "Revert Block" button reverts the change; with text selected, a "Revert Selection" button reverts only the selected changes. `revert(change, cx)`, `revert_range_mappings(changes, cx)` and the `Revert` action (the changes at the modified editor's cursor) do the same from code. `add_hunk_action` adds host buttons (Monaco's `DiffEditorHunkToolbar` and `DiffEditorSelectionToolbar` menus, e.g. Stage); they get a `DiffHunkContext` with the change and the original text with it applied. Up to 3 buttons show. The gutter is hidden when `render_gutter_menu` is off or there are no buttons (no host buttons and a read-only modified editor). Clicking it focuses the modified editor. Without it, `render_margin_revert_icon` shows Monaco's older `→` arrows in the modified editor's glyph margin instead.
- **Accessible diff viewer:** F7 (or `accessible_diff_viewer_next`) opens a list of the changes for screen reader and keyboard users, like Monaco's. Each change shows with three lines of context and both line numbers. F7 and Shift+F7 move between changes, Up and Down between lines, Enter or Space jumps to the line in its editor, and Escape closes the list. `only_show_accessible_diff_viewer` shows only the list. The list covers the editors (they get no clicks or wheel) and scrolls sideways for long lines. Moving onto an inserted or deleted line, `go_to_diff` and cursor moves onto a change emit `DiffEditorEvent::AccessibilitySignal` (`DiffLineInserted`, `DiffLineDeleted` or `DiffLineModified`), so a host can play a sound as VS Code does.
- **Moved blocks:** with `show_moves`, code that moved gets a border and a "Code moved to line …" caption on both sides, and a line from the original's scrollbar to the moved code in the modified editor joins the two places. The block at the cursor is highlighted. "Compare" on a caption (or `set_move_to_compare`) shows just that block's changes, aligned, with its two places scrolled side by side. Escape ends it.
- **Overview ruler and sash:** with `render_overview_ruler`, a 30px column at the right marks the original's changes in its left half and the modified's in its right half, with the modified editor's viewport as a slider to drag. The sash between the sides keeps each at least 100px wide; a double click (or `reset_sash`) puts it back at `split_view_default_ratio`.
- **Options:** `DiffEditorOptions` (Monaco `IDiffEditorOptions`):
  - `ignore_trim_whitespace`
  - `max_computation_time_ms`
  - `render_indicators` (`+`/`−` signs)
  - `render_overview_ruler`
  - `original_editable`
  - `enable_split_view_resizing` and `split_view_default_ratio` (drag the sash between the sides)
  - `show_moves` (detect and show moved blocks)
  - `hide_unchanged_regions` (`enabled`, `context_line_count`, `minimum_line_count`, `reveal_line_count`)
  - `render_gutter_menu` and `render_margin_revert_icon`
  - `only_show_accessible_diff_viewer`
  - `render_side_by_side`, `use_inline_view_when_space_is_limited` and `render_side_by_side_inline_breakpoint`
  - `compact_mode` (a diff of only insertions and deletions shows inline, and hidden regions get thin "N hidden lines" separators, none at the start or end)
  - `diff_word_wrap` (`On`, `Off` or `Inherit`: both editors' `word_wrap_override1`; the hidden original editor never wraps inline)
  - `diff_algorithm` (`Advanced`, the default, or `Legacy`, Monaco's older algorithm without moves)
  - `max_file_size` (MB, default 50, 0 for no limit): larger models show without a diff
  - `diff_code_lens` (off): code lenses in both editors (sets their `code_lens`)
  - `accessibility_verbose` (off): adds " use F7 to open the accessible diff viewer." to both editors' `aria_label` (Monaco names its accessibility help, which this port doesn't have)
  - `show_empty_decorations` (on, Monaco `experimental.showEmptyDecorations`): a 3px bar where characters were inserted or deleted when the other side has none
  - `use_true_inline_view` (off, Monaco `experimental.useTrueInlineView`): in the inline view, changes whose parts each stay on one line show the deleted text struck through before the inserted text, without the deleted lines above; `compact_mode` then also treats them as simple
  - `hide_original_line_numbers` (off): no original line number column in the inline view
  - `is_in_embedded_editor` (off): sets the `isEmbeddedDiffEditor` key context
  - `original_aria_label` and `modified_aria_label`: the editors' screen reader labels
- **Computing diffs directly:** `gpui_monaco::diff::compute_diff` is Monaco's diff algorithm on its own, for hosts that only need the changes; `compute_legacy_diff` is the legacy one.

Colors come from the theme: `diffEditor.insertedTextBackground`, `removedTextBackground`, `insertedLineBackground`, `removedLineBackground`, the `diffEditorGutter.*` backgrounds, `diffEditor.diagonalFill`, `diffEditor.border`, `diffEditorOverview.insertedForeground` and `removedForeground`, `diffEditor.move.border` and `moveActive.border`, and the `diffEditor.unchangedRegion*` and `unchangedCodeBackground` colors.

## Many files

`MultiDiffEditor` shows the diffs of many files in one scrolling list, like Monaco's multi-file diff editor. Each file is a `DiffEditor` under a header with its name, and each header sticks to the top while its file scrolls by. Unchanged regions are always hidden, and each diff is exactly as tall as what it shows.

```rust
use gpui_monaco::{MultiDiffDocument, MultiDiffEditor};

let documents = vec![
    MultiDiffDocument::new("src/main.rs", old_main, new_main),
    // A created file has no original; a deleted one no modified.
    MultiDiffDocument { label: "src/new.rs".into(), original_label: None, original: None, modified: Some(new_file), binary: false },
    // A binary file shows a "Binary file changed" placeholder; its models
    // only say which sides exist.
    MultiDiffDocument { label: "img/logo.png".into(), original_label: None, original: Some(empty_a), modified: Some(empty_b), binary: true },
];
let multi = cx.new(|cx| MultiDiffEditor::new(documents, window, cx));
```

- **Variants:** `set_variant` picks one of Monaco's three looks (`MultiDiffEditorVariant`). `NoCardsNonCompact`, the default and standalone Monaco's, has 40px headers with a border above. `NoCards` has compact 32px headers on the list's background with a line between files, diffs the whole width and 8px below each. `Cards` puts each file in a bordered, rounded card. In both compact variants clicking a header (or Enter or Space on a focused one) collapses its file, the toolbar shows only while the mouse is over the file or it has focus, the original side hides its line numbers, and the status letters use the git decoration colors.
- **Headers:** the chevron collapses a file to its header (`set_collapsed`, `set_all_collapsed`). Headers show the file name with its folder, and `A`, `D` or `R` for created, deleted and renamed (`original_label`) files. `add_file_action` adds a button to every header's toolbar (Monaco's `MultiDiffEditorFileToolbar`); its handler gets the file's index and document.
- **Binary files:** `binary: true` shows Monaco's placeholder instead of a diff. `set_open_binary_diff_handler` adds an "Open Diff" button to the placeholders of files with both sides (standalone Monaco has none). Navigation focuses the placeholder and moves past it.
- **Scrolling:** the list has Monaco's auto-hiding scrollbars, vertical and (for wide files) horizontal. A mouse wheel scrolls it smoothly over 100ms, a trackpad at once. When files change height (measured, collapsed or expanded), what's at the top of the list stays where it is (at the bottom while scrolling up), leaving room above the first file or below the last one if needed, as Monaco's compressed scroll view does; scrolling towards that room uses it up. Moving the cursor with the keyboard past the part of a file that shows scrolls the list along. `reveal(index)` scrolls a file's header to the top, and `reveal_range` centers one of its lines and highlights the range for 350ms. Horizontal scrolling moves every file together.
- **View state:** `save_view_state` gives the scroll position, each file's collapsed state and selections, and the active file (`MultiDiffEditorViewState`, keyed by the files' paths); `restore_view_state` puts them back on the files with the same paths, applying a scroll position once the list is tall enough for it.
- **Navigation:** `go_to_next_change` and `go_to_previous_change` move through the changes of all files, and `go_to_file` goes to a file's first change. Monaco's standalone multi diff editor binds no keys to these (VS Code's workbench does), so neither does `MultiDiffEditor`: bind them in your app. `active_item` is the file last focused or navigated to; its header and bottom border use `focusBorder`.
- **Editors:** like Monaco, only the expanded files in view have a `DiffEditor`, reused from a small pool as you scroll; a file keeps its selections and collapsed regions while it has none. `diff_editor(index)` gives a file's editor while it has one.
- **Options:** `update_options` applies `DiffEditorOptions` to every file.
- **Colors:** `multiDiffEditor.background`, `multiDiffEditor.headerBackground`, `multiDiffEditor.border`, `sideBarSectionHeader.border`, `focusBorder` and the `scrollbarSlider.*` colors; for the compact variants also `panel.border`, `list.hoverBackground`, `list.inactiveSelectionBackground`, `toolbar.hoverBackground`, `toolbar.activeBackground` and the `gitDecoration.*ResourceForeground` colors, and for cards `sideBarSectionHeader.background`, `widget.border` (else `editorWidget.border`) and `list.focusBackground`. Binary placeholders use `descriptionForeground` and the `button.secondary*` colors, and the reveal highlight `editor.symbolHighlightBackground`.

