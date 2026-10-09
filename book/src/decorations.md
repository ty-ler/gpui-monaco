# Decorations

Decorations (Monaco `deltaDecorations`) style ranges of text without changing it. Typical uses:

- search results
- diagnostics
- git gutter marks
- breakpoints
- inline hints

They live on the `TextModel`. Their ranges follow edits.

```rust,ignore
use gpui_monaco::{DecorationOptions, DecorationUnderline, InjectedText, OverviewRulerLane};

let ids = model.update(cx, |model, cx| {
    model.delta_decorations(
        None,          // owner: None shows them in every editor of the model
        &old_ids,      // decorations to replace (Monaco's oldDecorations)
        vec![
            (range.clone(), DecorationOptions {
                background: Some(hsla(0.15, 0.8, 0.5, 0.25)),
                overview_ruler: Some((yellow, OverviewRulerLane::Center)),
                hover_message: Some("**Note:** found here".into()),
                ..Default::default()
            }),
            (line_range, DecorationOptions {
                is_whole_line: true,
                glyph: Some(("●".into(), red)),   // glyph margin (enable `glyph_margin`)
                gutter_bar: Some(green),         // like a git change bar
                ..Default::default()
            }),
            (word_range, DecorationOptions {
                underline: Some(DecorationUnderline { color: blue, wavy: false }),
                after: Some(InjectedText {
                    content: ": i32".into(),
                    style: TokenStyle { foreground: Some(gray), italic: true, ..Default::default() },
                    background: None,
                }),
                ..Default::default()
            }),
        ],
        cx,
    )
});
```

Ranges are byte offsets. To convert from `PointUtf16`, use `model.buffer().point_utf16_to_offset(point)`.

## `DecorationOptions`

| Field | Monaco | Effect |
|---|---|---|
| `background`, `border` | `className` styling | fills or outlines the range |
| `underline` | `inlineClassName` with `text-decoration` | straight or wavy underline |
| `text` | `inlineClassName` | overrides the text's color, bold or italic |
| `is_whole_line` | `isWholeLine` | the background spans the full line width |
| `glyph` | `glyphMarginClassName` | a short glyph in the glyph margin |
| `line_number_color` | `lineNumberClassName` | colors the line number |
| `gutter_bar` | `linesDecorationsClassName` | a bar between the line numbers and the text |
| `overview_ruler` | `overviewRuler` | a mark in the scrollbar's overview ruler |
| `before`, `after` | `before`, `after` | text injected before or after the range; the cursor skips over it |
| `hover_message` | `hoverMessage` | markdown shown in the hover |
| `stickiness` | `stickiness` | whether typing at the edges grows the range |
| `z_index` | `zIndex` | paint order for overlapping decorations |

## Ownership

Pass the editor's entity id as `owner` to show decorations only in that editor (Monaco's `ownerId`). The editor uses this internally for find matches and bracket highlights, so a split view doesn't show the other editor's search. Use `None` for decorations every editor should show.

Look decorations up with `model.decorations_in_range(range, editor)`, `model.all_decorations(editor)` or `model.decoration(id)`. The `editor` argument filters by owner. Remove them by passing their ids as `old` with an empty `new`.
