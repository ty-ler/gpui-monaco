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
| `margin_background` | `marginClassName` | a background across the gutter on the decorated lines |
| `lines_decoration_text` | `linesDecorationsClassName` (icons) | text such as `+` in the line decorations area on each decorated line |
| `block` | `blockClassName` + `blockPadding` | a bordered box around the decorated lines and the view zones above them |
| `line_height` | `lineHeight` | the covered lines' height, as a multiple of the line height; text stays centered |
| `glyph` | `glyphMarginClassName` | a short glyph in the glyph margin, on every line of the range; the highest `z_index` wins a lane |
| `glyph_margin` | `glyphMargin` | the glyph's lane (`GlyphMarginLane::Left`, `Center` or `Right`), and `persist_lane` to keep the lane on every line; the margin is a line height wide per lane in use |
| `glyph_margin_hover_message` | `glyphMarginHoverMessage` | markdown shown when the mouse rests on the row's glyph margin or line number |
| `line_number_color` | `lineNumberClassName` | colors the line number |
| `gutter_bar` | `linesDecorationsClassName` | a bar between the line numbers and the text |
| `overview_ruler` | `overviewRuler` | a mark in the scrollbar's overview ruler, in a color and lane, spanning the range's lines |
| `minimap` (`MinimapDecoration`) | `minimap` | a mark in the minimap: `Inline` over the characters with a half-color band across the line, or a 2px `Gutter` bar; or a section header (`section_header_style`, `section_header_text`) |
| `before`, `after` | `before`, `after` | text injected before or after the range; the cursor skips over it |
| `hover_message` | `hoverMessage` | markdown shown in the hover |
| `stickiness` | `stickiness` | whether typing at the edges grows the range |
| `z_index` | `zIndex` | paint order for overlapping decorations |

## Collections

`editor.create_decorations_collection(decorations, cx)` returns a `DecorationsCollection` (Monaco `createDecorationsCollection`). It's a set of decorations that the editor owns, positioned with UTF-16 points:

- `set` replaces all of them, `append` adds more and `clear` removes them.
- `len`, `ids` and `has` describe the set.
- `range(i)` and `ranges()` give their current positions after edits.

It saves keeping the ids yourself:

```rust,ignore
let mut highlights = editor.update(cx, |editor, cx| editor.create_decorations_collection(vec![], cx));
highlights.set(vec![(start..end, options)], cx);
```

## Ownership

Pass the editor's entity id as `owner` to show decorations only in that editor (Monaco's `ownerId`). The editor uses this internally for find matches and bracket highlights, so a split view doesn't show the other editor's search. Use `None` for decorations every editor should show.

Look decorations up with `model.decorations_in_range(range, editor)`, `model.all_decorations(editor)` or `model.decoration(id)`. The `editor` argument filters by owner. Remove them by passing their ids as `old` with an empty `new`.
