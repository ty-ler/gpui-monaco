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

Markers are rendered as:

- **Squiggles:** red for errors, yellow for warnings and blue for info. Hints get no squiggle.
- **Overview ruler marks** for errors, warnings and infos, in the right lane, in the theme's `editorOverviewRuler.errorForeground`, `warningForeground` and `infoForeground`.
- **Tags:**
  - `MarkerTag::Unnecessary` fades the code (`show_unused`, using the theme's `editorUnnecessaryCode.opacity`).
  - `MarkerTag::Deprecated` strikes it through (`show_deprecated`).
  - Tagged hints get no squiggle.
- **Hovers:** the message in the editor font, then the source and code, then each `related_information` entry, whose `file(line, col): ` link opens the location or emits `CodeEditorEvent::OpenLocation`. With a `code_target` (Monaco's `code: { value, target }`), the code is a link to it, here and in the marker zone, opened through the link openers or `CodeEditorEvent::OpenLink`. The marker's range is highlighted (`editor.hoverHighlightBackground`). At the bottom, the status bar for the most severe marker: "View Problem" (errors, warnings and infos) opens the marker zone, and Quick Fix, computed from the quick fix code actions for the marker, says "Checking for quick fixes...", then "Quick Fix..." (the code action menu with them, below the link) or "No quick fixes available". Read-only editors get no Quick Fix.

`render_validation_decorations` hides all of this as Monaco does:
- `Editable` (the default) shows it unless the editor is read-only.
- `On` always shows it.
- `Off` never does.

Marker ranges follow edits. `model.markers()` returns the current ranges, sorted by position. As in Monaco, an empty range is widened to the character at its position, markers without a message are dropped, and an end before the start moves to the start's line.

## Markers across models

Monaco's global marker API works across every live model:

```rust,ignore
use gpui_monaco::MarkerFilter;

// Monaco `getModelMarkers`: any of owner, resource (the model's uri) and take.
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

Unlike `model.markers()`, `model_markers` returns markers as they were set, as Monaco's marker service does. Changes made in one update arrive as one event, each uri once (Monaco debounces it). Markers live in their model, so they go away with it; Monaco keeps markers for uris without a model except in-memory ones.

## Navigation

F8 and Shift+F8 (`GoToNextMarker` and `GoToPreviousMarker`) move to the next or previous problem. They show it in an inline zone below the line with "1 of 3 problems", the message and its related information, scrolled into view. Escape closes the zone.

As in Monaco, they visit errors, warnings and infos (not hints), most severe first and then by position. The first move takes the marker under the cursor, or the next one; later moves step through the list, so markers starting at the same place are each visited. When the host changes the markers while the zone is open, it shows the marker now at its place, or fades to italic when there's none.
