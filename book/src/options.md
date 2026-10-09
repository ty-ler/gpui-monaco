# Options

`EditorOptions` mirrors Monaco's `IEditorOptions`. Its defaults are Monaco's defaults. To change options, clone the current ones, edit the copy and apply it:

```rust,ignore
editor.update(cx, |editor, cx| {
    let mut options = editor.options().clone();
    options.word_wrap = WordWrap::On;
    options.minimap.enabled = false;
    options.font_size = px(13.);
    editor.update_options(options, cx);
});
```

| Field | Monaco option | Notes |
|---|---|---|
| `font_family`, `font_size`, `line_height` | `fontFamily`, `fontSize`, `lineHeight` | `line_height: None` derives it from the font size, as Monaco does |
| `line_numbers` | `lineNumbers` | `On`, `Off`, `Relative`, `Interval` |
| `line_numbers_min_chars`, `line_decorations_width`, `glyph_margin` | same names | gutter layout |
| `render_line_highlight` | `renderLineHighlight` | `None`, `Gutter`, `Line`, `All` |
| `cursor_style`, `cursor_width`, `cursor_blinking` | `cursorStyle`, `cursorWidth`, `cursorBlinking` | |
| `read_only` | `readOnly` | |
| `word_separators` | `wordSeparators` | used by word movement and whole-word find |
| `word_wrap`, `word_wrap_column`, `wrapping_indent` | `wordWrap`, `wordWrapColumn`, `wrappingIndent` | `Off`, `On`, `WordWrapColumn`, `Bounded` |
| `folding`, `show_folding_controls` | `folding`, `showFoldingControls` | |
| `scroll_beyond_last_line`, `mouse_wheel_scroll_sensitivity` | same names | |
| `vertical_scrollbar_size`, `horizontal_scrollbar_size` | `scrollbar.verticalScrollbarSize`, … | |
| `minimap` | `minimap` | `enabled`, `max_column`, `scale`, `always_show_slider` |
| `sticky_scroll` | `stickyScroll` | `enabled`, `max_line_count` |
| `auto_closing_brackets`, `auto_closing_quotes`, `auto_closing_comments` | `autoClosingBrackets`, … | `Always`, `LanguageDefined`, `BeforeWhitespace`, `Never` |
| `auto_closing_overtype`, `auto_closing_delete`, `auto_surround` | same names | |
| `auto_indent` | `autoIndent` | `None`, `Keep`, `Brackets`, `Advanced`, `Full` |
| `match_brackets` | `matchBrackets` | |
| `quick_suggestions` | `quickSuggestions` | completion while typing |
| `empty_selection_clipboard` | `emptySelectionClipboard` | copying with no selection copies the line |
| `sticky_tab_stops` | `stickyTabStops` | |
| `single_line` | — | a one-line input. Enter, Tab and Escape go to the parent, and pasted line breaks become spaces. |

Model-level settings live in `ModelOptions` (Monaco `ITextModelUpdateOptions`), set with `model.update_options(…)`:

- `tab_size`
- `indent_size`
- `insert_spaces`

Changing options takes effect on the next frame. Wrapping, folding and the minimap update incrementally.
