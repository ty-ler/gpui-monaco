# Diagnostics

Diagnostics are model markers, set per owner (Monaco `setModelMarkers`). Each owner, such as a linter or compiler, replaces its own markers without touching other owners'.

```rust,ignore
use gpui_monaco::{Marker, MarkerSeverity};

model.update(cx, |model, cx| {
    model.set_markers(
        "rustc",
        vec![Marker {
            severity: MarkerSeverity::Error,
            message: "mismatched types".into(),
            range: PointUtf16::new(10, 4)..PointUtf16::new(10, 14),
            source: Some("rustc".into()),
            code: Some("E0308".into()),
            code_target: Some("https://doc.rust-lang.org/error_codes/E0308.html".into()),
            ..Default::default()
        }],
        cx,
    );
});
```

Markers belong to the model's uri, not the model: they outlive it and decorate the next model with the uri. `set_markers_for_uri(cx, owner, uri, markers)` sets them for a uri that has no model yet (Monaco's `IMarkerService.changeOne`), as the LSP bridge does for diagnostics of files that aren't open; `model_markers` lists them, and a model opened later shows them at once.

Markers are rendered as:

- **Squiggles:** red for errors, yellow for warnings and blue for info, in the theme's `editorError.foreground` and the like. High contrast themes also draw Monaco's double underline (`editorError.border`, `editorWarning.border`, `editorInfo.border`).
- **Hint dots:** a hint gets Monaco's three dots under its first two characters (`editorHint.foreground`), or a dotted underline (`editorHint.border`) in high contrast.
- **Overview ruler marks** for errors, warnings and infos, in the right lane, in the theme's `editorOverviewRuler.errorForeground`, `warningForeground` and `infoForeground`.
- **Tags:**
  - `MarkerTag::Unnecessary` fades the code (`show_unused`, using the theme's `editorUnnecessaryCode.opacity`).
  - `MarkerTag::Deprecated` strikes it through (`show_deprecated`).
  - Tagged hints get no dots.
- **Hovers:** the message in the editor font, then the source and code, then each `related_information` entry, whose `file(line, col): ` link opens the location or emits `CodeEditorEvent::OpenLocation`. With a `code_target` (Monaco's `code: { value, target }`), the code is a link to it, here and in the marker zone, opened through the link openers, in the browser for web links, or else reported as `CodeEditorEvent::OpenLink`. The marker's range is highlighted (`editor.hoverHighlightBackground`). At the bottom, the status bar for the most severe marker: "View Problem" (errors, warnings and infos) opens the marker zone, and Quick Fix, computed from the quick fix code actions for the marker, says "Checking for quick fixes...", then "Quick Fix..." (the code action menu with them, below the link) or "No quick fixes available". Read-only editors get no Quick Fix.

`render_validation_decorations` hides all of this as Monaco does:
- `Editable` (the default) shows it unless the editor is read-only.
- `On` always shows it.
- `Off` never does.

Marker ranges follow edits. `model.markers()` returns the current ranges, sorted by position. As in Monaco (`_createDecorationRange`):
- An empty range covers the word at its position. On an empty line or after the line's last non-whitespace character it stays empty and is drawn one character wide.
- A whole-line marker, from column 0 to `u32::MAX` (Monaco's `Number.MAX_VALUE`), starts at the line's first non-whitespace character.
- Markers without a message are dropped, and an end before the start moves to the start's line.
- Only the first 500 markers of a model are drawn (`MAX_DECORATED_MARKERS`).

## Markers across models

Monaco's global marker API works across every live model:

```rust,ignore
use gpui_monaco::MarkerFilter;

// Monaco `getModelMarkers`: any of owner, resource (a model's `resource()`) and take.
let errors = gpui_monaco::model_markers(cx, &MarkerFilter {
    owner: Some("rustc".into()),
    ..Default::default()
});
for m in errors {
    println!("{:?} {}: {}", m.resource, m.owner, m.marker.message);
}

// Monaco `removeAllMarkers`.
gpui_monaco::remove_all_markers(cx, "rustc");

// Monaco `onDidChangeMarkers`: the uris whose markers changed.
gpui_monaco::on_did_change_markers(cx, |uris, cx| refresh_problems_panel(uris, cx)).detach();
```

Unlike `model.markers()`, `model_markers` returns markers as they were set, as Monaco's marker service does. Changes made in one update arrive as one event, each uri once (Monaco debounces it).

As in Monaco, markers belong to the uri rather than the model. A model without a uri has Monaco's `inmemory://model/N` as its `resource()`, so its markers are reported and filtered like any other's. When a model is released, its markers stay for the next model with that uri, unless the uri is `inmemory:`, `internal:` or `vscode:`.

## Navigation

Alt+F8 and Shift+Alt+F8 (`GoToNextMarker` and `GoToPreviousMarker`, or `CodeEditor::go_to_marker`) move to the next or previous problem in the model. They show it in an inline zone below the line with "1 of 3 problems", the message and its related information, scrolled into view. The zone's header has the next and previous buttons (these two actions) and close. Escape closes the zone.

F8 and Shift+F8 (`GoToNextMarkerInFiles` and `GoToPreviousMarkerInFiles`, or `CodeEditor::go_to_marker_in_files`) do the same over every uri's markers, as Monaco's "in Files" actions do: by uri, then as below, and the zone counts them all. When the next one is in another uri, the zone closes and the location goes to your editor openers (`register_editor_opener`, see [Events](./model-and-view.md#events)) with the marker's range, or to `CodeEditorEvent::OpenLocation` when none opens it. If the opener sets the editor's model to that uri, the zone opens there on the marker:

```rust,ignore
// Keep the `Disposable` to unregister it.
let opener = gpui_monaco::register_editor_opener(cx, move |editor, uri, selection, cx| {
    let Some(model) = models.get(uri).cloned() else { return false };
    editor.update(cx, |editor, cx| {
        editor.set_model(model, cx);
        if let Some(range) = selection {
            editor.set_selections(&[Selection::collapsed(range.start)], cx);
        }
    });
    true
});
```

As in Monaco, they visit errors, warnings and infos (not hints), most severe first and then by position. The first move takes the marker under the cursor, or the next one; later moves step through the list, so markers starting at the same place are each visited. When the host changes the markers while the zone is open, it shows the marker now at its place, or fades to italic when there's none.
