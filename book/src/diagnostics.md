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
            ..Default::default()
        }],
        cx,
    );
});
```

Markers are rendered as:

- **Squiggles:** red for errors, yellow for warnings and blue for info. Hints get no squiggle.
- **Overview ruler marks** for warnings and errors.
- **Tags:**
  - `MarkerTag::Unnecessary` fades the code (`show_unused`, using the theme's `editorUnnecessaryCode.opacity`).
  - `MarkerTag::Deprecated` strikes it through (`show_deprecated`).
  - Tagged hints get no squiggle.
- **Hovers:** the message, then the source and code, then the `related_information` links, which open the location or emit `CodeEditorEvent::OpenLocation`. Below them, "View Problem" opens the marker zone and "Quick Fix..." runs the code actions.

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

F8 and Shift+F8 (`GoToNextMarker` and `GoToPreviousMarker`) move to the next or previous problem. They show it in an inline zone below the line with "1 of 3 problems", the message and its related information. Escape closes the zone.
