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
| `cursor_style`, `cursor_width`, `cursor_height` | `cursorStyle`, `cursorWidth`, `cursorHeight` | `cursor_height` 0 is the line height |
| `cursor_blinking` | `cursorBlinking` | `Blink`, `Smooth`, `Phase`, `Expand`, `Solid` (read-only editors don't blink) |
| `cursor_smooth_caret_animation` | `cursorSmoothCaretAnimation` | `Off`, `Explicit` (keys and clicks, not typing), `On` |
| `cursor_surrounding_lines`, `cursor_surrounding_lines_style` | same names | lines kept around the cursor; `All` applies them to mouse moves too |
| `smooth_scrolling` | `smoothScrolling` | animate reveals, cursor moves and mouse wheels (off) |
| `scroll_beyond_last_column`, `reveal_horizontal_right_padding` | same names | room past the longest line (4 columns) and right of revealed positions (15px) |
| `fast_scroll_sensitivity`, `scroll_predominant_axis` | same names | Alt+wheel speed (5); scroll along the main axis only (on) |
| `scroll_on_middle_click` | `scrollOnMiddleClick` | a middle click scrolls toward the mouse until the next click (off) |
| `padding` | `padding` | `top` and `bottom` space |
| `rounded_selection`, `render_line_highlight_only_when_focus` | same names | (on, off) |
| `read_only` | `readOnly` | |
| `word_separators` | `wordSeparators` | used by word movement and whole-word find |
| `word_wrap`, `word_wrap_column`, `wrapping_indent` | `wordWrap`, `wordWrapColumn`, `wrappingIndent` | `Off`, `On`, `WordWrapColumn`, `Bounded` |
| `folding`, `show_folding_controls` | `folding`, `showFoldingControls` | |
| `scroll_beyond_last_line`, `mouse_wheel_scroll_sensitivity` | same names | |
| `vertical_scrollbar_size`, `horizontal_scrollbar_size` | `scrollbar.verticalScrollbarSize`, … | |
| `scrollbar` | `scrollbar.*` | `vertical`/`horizontal` (`Auto` shows while hovered or just scrolled, `Visible`, `Hidden`), `use_shadows`, `*_has_arrows`, `arrow_size`, `*_slider_size`, `handle_mouse_wheel`, `always_consume_mouse_wheel`, `scroll_by_page` |
| `overview_ruler_lanes`, `overview_ruler_border`, `hide_cursor_in_overview_ruler` | same names | lanes (3; 0 hides the marks), a left border, the cursor marks |
| `minimap` | `minimap` | `enabled`, `max_column`, `scale`, `always_show_slider`, `side` (right or left), `render_characters`, `size` (`Proportional`, `Fill`, `Fit`) |
| `sticky_scroll` | `stickyScroll` | `enabled`, `max_line_count`, `default_model`, `scroll_with_editor` |
| `auto_closing_brackets`, `auto_closing_quotes`, `auto_closing_comments` | `autoClosingBrackets`, … | `Always`, `LanguageDefined`, `BeforeWhitespace`, `Never` |
| `auto_closing_overtype`, `auto_closing_delete`, `auto_surround` | same names | |
| `auto_indent` | `autoIndent` | `None`, `Keep`, `Brackets`, `Advanced`, `Full` |
| `match_brackets` | `matchBrackets` | |
| `code_lens`, `code_lens_font_family`, `code_lens_font_size` | `codeLens`, … | show code lenses (on); their font (the editor's; 90% of its size) |
| `linked_editing` | `linkedEditing` | mirror edits between linked ranges, like HTML tag pairs (off) |
| `inline_suggest` | `inlineSuggest.*` | `enabled` (on), `mode` (`SubwordSmart`), `suppress_suggestions`, `keep_on_blur`, `min_show_delay`, `suppress_in_snippet_mode` (on) |
| `semantic_highlighting` | `semanticHighlighting.enabled` | `On`, `Off` or `ConfiguredByTheme` (the theme's `semanticHighlighting`) |
| `links` | `links` | detect links and follow them with Cmd/Ctrl+click (on) |
| `color_decorators` | `colorDecorators` | swatches before colors (on) |
| `color_decorators_limit` | `colorDecoratorsLimit` | at most this many swatches (500) |
| `color_decorators_activated_on` | `colorDecoratorsActivatedOn` | what opens the color picker from a swatch |
| `default_color_decorators` | `defaultColorDecorators` | when the built-in color finder runs (auto) |
| `quick_suggestions` | `quickSuggestions` | completion while typing, per token type: `other` (on), `comments`, `strings` |
| `quick_suggestions_delay` | `quickSuggestionsDelay` | wait before quick suggestions (10ms) |
| `suggest_on_trigger_characters` | `suggestOnTriggerCharacters` | open on provider trigger characters (on) |
| `accept_suggestion_on_enter` | `acceptSuggestionOnEnter` | `On`, `Smart` (Enter types a new line when the word is already complete) or `Off` |
| `accept_suggestion_on_commit_character` | `acceptSuggestionOnCommitCharacter` | (on) |
| `suggest_selection` | `suggestSelection` | `First`, `RecentlyUsed` or `RecentlyUsedByPrefix` |
| `word_based_suggestions` | `wordBasedSuggestions` | words of the current, matching-language or all registered documents when providers have nothing |
| `snippet_suggestions` | `snippetSuggestions` | where snippets sort; `None` hides them |
| `suggest` | `suggest.*` | `insert_mode`, `filter_graceful`, `match_on_word_start_only`, `show_deprecated`, `hidden_kinds`, `locality_bonus`, `font_size`, `line_height`, `show_icons`, `show_status_bar`, `show_inline_details` |
| `tab_completion` | `tabCompletion` | `Off`, `On` (Tab inserts the best suggestion and cycles through the others) or `OnlySnippets` (like `Off` until snippets land) |
| `inlay_hints` | `inlayHints` | whether inlay hints show (`On`, `OnUnlessPressed`, `OffUnlessPressed`, `Off`), padding and the per-line maximum length |
| `folding_strategy` | `foldingStrategy` | `Auto` (providers, then syntax, then indentation) or `Indentation` |
| `folding_maximum_regions` | `foldingMaximumRegions` | at most this many provider or syntax fold ranges (5000) |
| `folding_highlight` | `foldingHighlight` | shade the first line of collapsed regions (on) |
| `unfold_on_click_after_end_of_line` | `unfoldOnClickAfterEndOfLine` | unfold when clicking after a collapsed line (off) |
| `folding_imports_by_default` | `foldingImportsByDefault` | collapse import ranges at first (off) |
| `smart_select` | `smartSelect` | expand selection over leading/trailing whitespace lines and word parts (both on) |
| `lightbulb` | `lightbulb.enabled` | when the code action lightbulb asks for actions: `Off`, `OnCode` (default) or `On` |
| `goto_location` | `gotoLocation` | what go to definition, references and the other navigation commands do with several results, and their alternative commands |
| `format_on_type` | `formatOnType` | format with the on-type formatter after its trigger characters (off) |
| `format_on_paste` | `formatOnPaste` | format pasted text with the range formatter (off) |
| `empty_selection_clipboard` | `emptySelectionClipboard` | copying with no selection copies the line |
| `sticky_tab_stops` | `stickyTabStops` | |
| `multi_cursor_modifier`, `multi_cursor_paste`, `multi_cursor_limit`, `multi_cursor_merge_overlapping` | `multiCursorModifier`, … | Alt or Cmd/Ctrl to add cursors; spread or full paste |
| `column_selection` | `columnSelection` | mouse drags and Shift+arrows make column selections |
| `overtype_cursor_style`, `overtype_on_paste` | `overtypeCursorStyle`, `overtypeOnPaste` | overtype mode (Insert) |
| `comments` | `comments` | `insert_space`, `ignore_empty_lines` |
| `render_whitespace`, `render_control_characters`, `render_final_newline` | same names | `·` and `→` for whitespace; `␇` for control characters |
| `guides`, `rulers` | `guides`, `rulers` | indent guides (with the active one highlighted), bracket pair guides (`bracket_pairs`: off, active or on, plus horizontal ones) and vertical rulers |
| `bracket_pair_colorization` | `bracketPairColorization.enabled` | color brackets by nesting level |
| `single_line` | — | a one-line input. Enter, Tab and Escape go to the parent, and pasted line breaks become spaces. |

Model-level settings live in `ModelOptions` (Monaco `ITextModelUpdateOptions`), set with `model.update_options(…)`:

- `tab_size`
- `indent_size`
- `insert_spaces`

Changing options takes effect on the next frame. Wrapping, folding and the minimap update incrementally.
