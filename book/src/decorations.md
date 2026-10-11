# Decorations

Decorations (Monaco `deltaDecorations`) style ranges of text without changing it. Typical uses:

- search results
- diagnostics
- git gutter marks
- breakpoints
- inline hints

They live on the `TextModel`. Their ranges follow edits.

```rust,ignore
use gpui_monaco::{
    DecorationColor, DecorationOptions, DecorationUnderline, InjectedText, OverviewRulerDecoration,
    OverviewRulerLane,
};

let ids = model.update(cx, |model, cx| {
    model.delta_decorations(
        None,          // owner: None shows them in every editor of the model
        &old_ids,      // decorations to replace (Monaco's oldDecorations)
        vec![
            (range.clone(), DecorationOptions {
                background: Some(hsla(0.15, 0.8, 0.5, 0.25)),
                // A theme color id follows theme changes (Monaco `ThemeColor`).
                overview_ruler: Some(OverviewRulerDecoration::new(
                    DecorationColor::theme("editorOverviewRuler.findMatchForeground"),
                    OverviewRulerLane::Center,
                )),
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
                    ..Default::default()
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
| `first_line_decoration` (`LinesDecoration`) | `firstLineDecorationClassName` | a bar or text in the line decorations area, only on the first view line of a wrapped line |
| `overview_ruler` (`OverviewRulerDecoration`) | `overviewRuler` | a mark in the scrollbar's overview ruler, in a color (`dark_color` in dark themes) and lane, spanning the range's lines |
| `minimap` (`MinimapDecoration`) | `minimap` | a mark in the minimap: `Inline` over the characters with a half-color band across the line, or a 2px `Gutter` bar; or a section header (`section_header_style`, `section_header_text`) |
| `before`, `after` | `before`, `after` | text injected before or after the range (see below) |
| `show_if_collapsed` | `showIfCollapsed` | an empty range still shows: its injected text only shows with it, and its background is widened to a character's width |
| `font_family`, `font_weight`, `font_style` | `fontFamily`, `fontWeight`, `fontStyle` | the decorated text's font; line wrapping still measures the editor's font |
| `hover_message` | `hoverMessage` | markdown shown in the hover |
| `stickiness` | `stickiness` | whether typing at the edges grows the range |
| `z_index` | `zIndex` | paint order for overlapping decorations |

Monaco's class names become styles here, so `className` effects beyond these fields can't be expressed. `inlineClassNameAffectsLetterSpacing` has no equivalent: it tells Monaco's DOM renderer to measure again, and text here is always shaped with its styles. A decoration doesn't change the font size (`fontSize`); `get_font_size_at_position` is the editor's.

### Theme colors

Overview ruler and minimap colors are `DecorationColor`s: a resolved `Hsla` (`.into()` converts one) or `DecorationColor::theme(id)`, a theme color id (Monaco `ThemeColor`) looked up in the editor's theme when drawn, so the mark follows theme changes. `Theme::color(id)` resolves any of Monaco's color ids, with its defaults.

### Injected text

`before` and `after` (`InjectedText`) show text at the range's start and end that isn't part of the model. On an empty range they show only with `show_if_collapsed`, as in Monaco.

- `style` colors the text, and `tokens` styles parts of it (Monaco's internal `tokens`, as ghost text uses), by byte ranges of `content`.
- `font_family`, `font_size`, `background`, `swatch` and `text_box` (margins, padding and corners) shape and box it, as inlay hints and color decorators do.
- `attached_data` (`AttachedData`) is the host's data, read back from the mouse target over the text (`MouseTarget::injected_text`) and from `injected_text_decorations`.
- `cursor_stops` (`InjectedTextCursorStops`, Monaco `cursorStops`) says where the cursor may show around the text. The cursor never stops inside it: its model position is the same on both sides. From the model (typing, `set_selections`) the cursor shows before the text. Moving onto the text with Left, Right, Down, End or a click puts the cursor after it with `Both` or `Right`, and before it with `Left` or `None`; Up puts it before it. A click on the text goes to its nearer side the stops allow. Inlay hints stop only after the hint, and inline suggestions only before.

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
