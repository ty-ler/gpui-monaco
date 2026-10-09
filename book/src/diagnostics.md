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
        }],
        cx,
    );
});
```

Markers are rendered as:

- **Squiggles:** red for errors, yellow for warnings and blue for info. Hints get no squiggle.
- **Overview ruler marks** for warnings and errors.
- **Hover messages:** the message, followed by the source and code.

Marker ranges follow edits. `model.markers()` returns the current ranges, sorted by position. As in Monaco, an empty range is widened to the character at its position.

## Navigation

F8 and Shift+F8 (`GoToNextMarker` and `GoToPreviousMarker`) move to the next or previous problem. They show it in an inline zone below the line with "1 of 3 problems" and the message. Escape closes the zone.
