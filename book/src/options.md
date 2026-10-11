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
| `font_weight`, `font_ligatures` | `fontWeight`, `fontLigatures` | ligatures are `Off` (default), `On`, or CSS `font-feature-settings` |
| `mouse_wheel_zoom` | `mouseWheelZoom` | Cmd+wheel (Ctrl elsewhere) zooms every editor's font; see `set_editor_zoom_level` |
| `line_numbers` | `lineNumbers` | `On`, `Off`, `Relative`, `Interval` (every 10th, the current and the last line), or `LineNumbers::custom(\|n\| …)` for Monaco's function: a label per 1-based line, empty for none |
| `select_on_line_numbers` | `selectOnLineNumbers` | clicking a line number selects the line (on); other gutter clicks never select |
| `line_numbers_min_chars`, `line_decorations_width`, `glyph_margin` | same names | gutter layout |
| `render_line_highlight` | `renderLineHighlight` | `None`, `Gutter`, `Line`, `All` |
| `cursor_style`, `cursor_width`, `cursor_height` | `cursorStyle`, `cursorWidth`, `cursorHeight` | `Line`, `Block`, `Underline`, `LineThin`, `BlockOutline`, `UnderlineThin`; `cursor_height` 0 is the line height |
| `mouse_style` | `mouseStyle` | the mouse cursor over the text: `Text` (I-beam), `Default` (arrow) or `Copy` |
| `mouse_middle_click_action` | `mouseMiddleClickAction` | `Default`, `OpenLink` (a middle click opens a link) or `CtrlLeftClick` (it acts like Cmd/Ctrl+click) |
| `cursor_blinking` | `cursorBlinking` | `Blink`, `Smooth`, `Phase`, `Expand`, `Solid` (read-only editors don't blink) |
| `cursor_smooth_caret_animation` | `cursorSmoothCaretAnimation` | `Off`, `Explicit` (keys and clicks, not typing), `On` |
| `cursor_surrounding_lines`, `cursor_surrounding_lines_style` | same names | lines kept around the cursor; `All` applies them to mouse moves too |
| `smooth_scrolling` | `smoothScrolling` | animate reveals, cursor moves and mouse wheels (off) |
| `scroll_beyond_last_column`, `reveal_horizontal_right_padding` | same names | room past the longest line (4 columns) and right of revealed positions (15px) |
| `fast_scroll_sensitivity`, `scroll_predominant_axis` | same names | Alt+wheel speed (5); scroll along the main axis only (on) |
| `scroll_on_middle_click` | `scrollOnMiddleClick` | a middle click scrolls toward the mouse until the next click (off) |
| `inertial_scroll` | `inertialScroll` | wheel and trackpad scrolling keeps going and slows down after the events stop, as Monaco's scrollable element does (off; mostly for Linux touchpads, since macOS and Windows already add momentum) |
| `padding` | `padding` | `top` and `bottom` space |
| `rounded_selection`, `render_line_highlight_only_when_focus` | same names | (on, off) |
| `read_only` | `readOnly` | edits are refused and show `read_only_message` |
| `read_only_message` | `readOnlyMessage` | markdown; defaults to "Cannot edit in read-only editor" |
| `placeholder` | `placeholder` | shown while the model is empty |
| `unicode_highlight` | `unicodeHighlight.*` | `non_basic_ascii`, `invisible_characters`, `ambiguous_characters`, `include_comments`, `include_strings`, `allowed_characters`, `allowed_locales` |
| `drag_and_drop`, `double_click_selects_block` | `dragAndDrop`, `doubleClickSelectsBlock` | drag the selection to move it (Alt/Ctrl copies); double-click inside brackets or quotes selects their content |
| `auto_indent_on_paste`, `auto_indent_on_paste_within_string` | `autoIndentOnPaste`, `autoIndentOnPasteWithinString` | re-indent pasted lines (off by default; needs `auto_indent: Full`) |
| `selection_clipboard` | `selectionClipboard` | Linux primary selection: selected text is copied to it, and a middle click pastes it (default on) |
| `stop_rendering_line_after` | `stopRenderingLineAfter` | draw at most this many characters of a line, then "Show more" (default 10 000) |
| `aria_label`, `aria_required`, `accessibility_page_size` | `ariaLabel`, `ariaRequired`, `accessibilityPageSize` | the screen reader name and how many lines it can read around the selection (500) |
| `accessibility_support` | `accessibilitySupport` | `Auto` (optimized while assistive technology is connected), `On` or `Off`. Optimized, wrapping is measured without continuation indent, and a model mostly made of lines of 10 000+ characters wraps at the viewport (unless `word_wrap_override1` is set); a diff editor stays side by side when narrow. `Off` gives the screen reader no text, only "The editor is not accessible at this time." |
| `screen_reader_announce_inline_suggestion` | `screenReaderAnnounceInlineSuggestion` | screen readers announce new ghost text, or where a next edit suggestion is (on); `CodeEditor::announce` announces anything |
| `tab_focus_mode` | `tabFocusMode` | Tab moves focus instead of indenting; `set_tab_focus_mode` (Ctrl+M) turns it on for every editor |
| `drop_into_editor`, `paste_as` | `dropIntoEditor`, `pasteAs` | `enabled` and when to show the button that switches edits |
| `unusual_line_terminators` | `unusualLineTerminators` | `Prompt` (default), `Auto` or `Off` for LS and PS characters |
| `word_separators` | `wordSeparators` | used by word movement and whole-word find |
| `word_segmenter_locales` | `wordSegmenterLocales` | BCP 47 tags (`ja`, `zh-CN`, `th`, …) whose words word movement, deletion and double-click selection find with ICU4X's word segmenter, for languages written without spaces (empty by default). The first valid tag is used; invalid ones are ignored. Needs the `word-segmenter` cargo feature (on by default) |
| `word_wrap`, `word_wrap_column`, `wrapping_indent` | `wordWrap`, `wordWrapColumn`, `wrappingIndent` | `Off`, `On`, `WordWrapColumn`, `Bounded` |
| `word_wrap_indicator` | `wordWrapIndicator` | a `↩` (theme color `editorWordWrapIndicator.foreground`, defaulting to the whitespace color) at the wrapping column of each soft-wrapped line; wrapping at the viewport then stops a column earlier (off) |
| `word_wrap_override1`, `word_wrap_override2` | `wordWrapOverride1`, `wordWrapOverride2` | `On`, `Off` or `Inherit` (default); the second overrides the first, which overrides `word_wrap`. `EditorOptions::effective_word_wrap` resolves them. The diff editor sets the first from `diff_word_wrap` |
| `word_wrap_break_before_characters`, `word_wrap_break_after_characters` | `wordWrapBreakBeforeCharacters`, `wordWrapBreakAfterCharacters` | where wrapping may break (Monaco's defaults) |
| `word_break` | `wordBreak` | `Normal`, or `KeepAll` to not break between CJK characters |
| `wrap_on_escaped_line_feeds` | `wrapOnEscapedLineFeeds` | while wrapping, a line with a quote also breaks after each literal `\n` (off) |
| `wrapping_strategy` | `wrappingStrategy` | `Simple` counts monospace columns; `Advanced` measures characters, for proportional fonts |
| `fullwidth_character_width` | `fullwidthCharacterWidth` | `Font` (default) or `TwoCells`: with a monospace font, full-width characters are centered in two cells, so columns line up; carets, clicks and measured wrapping follow the cells. Characters drawn with several glyphs and right-to-left lines keep the font's width, and so does ghost text |
| `folding`, `show_folding_controls` | `folding`, `showFoldingControls` | |
| `scroll_beyond_last_line`, `mouse_wheel_scroll_sensitivity` | same names | |
| `allow_variable_line_heights` | `allowVariableLineHeights` | decorations' `line_height` applies (default on) |
| `vertical_scrollbar_size`, `horizontal_scrollbar_size` | `scrollbar.verticalScrollbarSize`, … | |
| `scrollbar` | `scrollbar.*` | `vertical`/`horizontal` (`Auto` shows while hovered or just scrolled, `Visible`, `Hidden`), `use_shadows`, `*_has_arrows`, `arrow_size`, `*_slider_size`, `handle_mouse_wheel`, `always_consume_mouse_wheel`, `scroll_by_page`, `ignore_horizontal_scrollbar_in_content_height` (without `scroll_beyond_last_line`, a shown horizontal scrollbar adds its height to the content height so the last line can scroll above it; this leaves it out) |
| `overview_ruler_lanes`, `overview_ruler_border`, `hide_cursor_in_overview_ruler` | same names | lanes (3; 0 hides the marks), a left border, the cursor marks |
| `minimap` | `minimap` | `enabled`, `max_column`, `scale`, `always_show_slider`, `side` (right or left), `render_characters`, `size` (`Proportional`, `Fill`, `Fit`), section headers (`show_region_section_headers`, `show_mark_section_headers`, `mark_section_header_regex`, `section_header_font_size`, `section_header_letter_spacing`), `autohide` (`None`; `Mouseover` shows it while hovered; `Scroll` shows it while scrolling and lets clicks through while hidden) |
| `sticky_scroll` | `stickyScroll` | `enabled`, `max_line_count`, `default_model`, `scroll_with_editor` |
| `auto_closing_brackets`, `auto_closing_quotes`, `auto_closing_comments` | `autoClosingBrackets`, … | `Always`, `LanguageDefined`, `BeforeWhitespace`, `Never` |
| `auto_closing_overtype`, `auto_closing_delete`, `auto_surround` | same names | |
| `auto_indent` | `autoIndent` | `None`, `Keep`, `Brackets`, `Advanced`, `Full` |
| `match_brackets` | `matchBrackets` | `Always` (the pair at the cursor, or else the enclosing pair), `Near` or `Never` |
| `code_lens`, `code_lens_font_family`, `code_lens_font_size` | `codeLens`, … | show code lenses (on); their font (the editor's; 90% of its size) |
| `linked_editing` | `linkedEditing` | mirror edits between linked ranges, like HTML tag pairs (off) |
| `inline_suggest` | `inlineSuggest.*` | `enabled` (on), `mode` (`SubwordSmart`), `suppress_suggestions`, `keep_on_blur`, `min_show_delay`, `suppress_in_snippet_mode` (on), `show_toolbar` (`OnHover`, `Always`, `Never`), `syntax_highlighting_enabled` (on), `font_family` (the editor's), `edits` (inline edits: `enabled` (on), `show_collapsed` (off), `render_side_by_side` (`Auto`, `Never`), `allow_code_shifting` (`Always`, `Horizontal`, `Never`), `show_long_distance_hint` (on), `long_distance_hint_context_line_count` (0, up to 10)) |
| `semantic_highlighting` | `semanticHighlighting.enabled` | `On`, `Off` or `ConfiguredByTheme` (the theme's `semanticHighlighting`) |
| `links` | `links` | detect links and follow them with Cmd/Ctrl+click (on) |
| `color_decorators` | `colorDecorators` | swatches before colors (on) |
| `color_decorators_limit` | `colorDecoratorsLimit` | at most this many swatches (500) |
| `color_decorators_activated_on` | `colorDecoratorsActivatedOn` | what opens the color picker from a swatch |
| `default_color_decorators` | `defaultColorDecorators` | when the built-in color finder runs (auto) |
| `quick_suggestions` | `quickSuggestions` | completion while typing, per token type (`other`, `comments`, `strings`): `On` (the suggest widget), `Inline` (the items as ghost text), `Off`, or `OffWhenInlineCompletions` (the widget unless ghost text shows; the default for `other`, the others are `Off`) |
| `quick_suggestions_delay` | `quickSuggestionsDelay` | wait before quick suggestions (10ms) |
| `suggest_on_trigger_characters` | `suggestOnTriggerCharacters` | open on provider trigger characters (on) |
| `accept_suggestion_on_enter` | `acceptSuggestionOnEnter` | `On`, `Smart` (Enter types a new line when the word is already complete) or `Off` |
| `accept_suggestion_on_commit_character` | `acceptSuggestionOnCommitCharacter` | (on) |
| `suggest_selection` | `suggestSelection` | `First`, `RecentlyUsed` or `RecentlyUsedByPrefix` |
| `word_based_suggestions` | `wordBasedSuggestions` | words of the current, matching-language or all registered documents when providers have nothing |
| `snippet_suggestions` | `snippetSuggestions` | where snippets sort; `None` hides them |
| `suggest` | `suggest.*` | `insert_mode`, `filter_graceful`, `match_on_word_start_only`, `show_deprecated`, `hidden_kinds`, `locality_bonus`, `font_size`, `line_height`, `show_icons`, `show_status_bar`, `show_inline_details`, `preview`, `preview_mode`, `selection_mode` (whether a suggestion is focused when the list opens by itself: `Always`, `Never`, `WhenTriggerCharacter` or `WhenQuickSuggestion`), `snippets_prevent_quick_suggestions` (off) |
| `tab_completion` | `tabCompletion` | `Off`, `On` (Tab inserts the best suggestion and cycles through the others) or `OnlySnippets` (like `Off` until snippets land) |
| `inlay_hints` | `inlayHints` | whether inlay hints show (`On`, `OnUnlessPressed`, `OffUnlessPressed`, `Off`), padding, the per-line maximum length, and `font_size` (0, under 5 or above the editor's uses the editor's) and `font_family` |
| `parameter_hints` | `parameterHints` | `enabled`: open parameter hints while typing (on); `cycle`: going past the last or first signature wraps around instead of closing them (on) |
| `folding_strategy` | `foldingStrategy` | `Auto` (providers, then syntax, then indentation) or `Indentation` |
| `folding_maximum_regions` | `foldingMaximumRegions` | at most this many provider or syntax fold ranges (5000) |
| `folding_highlight` | `foldingHighlight` | shade the first line of collapsed regions (on) |
| `unfold_on_click_after_end_of_line` | `unfoldOnClickAfterEndOfLine` | unfold when clicking after a collapsed line (off) |
| `folding_imports_by_default` | `foldingImportsByDefault` | collapse import ranges at first (off) |
| `smart_select` | `smartSelect` | expand selection over leading/trailing whitespace lines and word parts (both on) |
| `lightbulb` | `lightbulb.enabled` | when the code action lightbulb asks for actions: `Off`, `OnCode` (default) or `On` |
| `goto_location` | `gotoLocation` | what go to definition, references and the other navigation commands do with several results, and their alternative commands |
| `peek_widget_default_focus` | `peekWidgetDefaultFocus` | what a new peek view focuses: `Tree` (the results list, default) or `Editor` (the preview) |
| `definition_link_opens_in_peek` | `definitionLinkOpensInPeek` | Cmd/Ctrl+click on a symbol shows its definitions in a peek view (off) |
| `find` | `find.*` | `cursor_move_on_type` (on), `seed_search_string_from_selection`, `auto_find_in_selection`, `add_extra_space_on_top` (on), `loop_` (on), `history` and `replace_history` (on), `close_on_result` (off), `find_on_type` (search while typing; when off, Enter searches; on), `global_find_clipboard` (share the search string through the find pasteboard on macOS, or between the app's editors; off) |
| `format_on_type` | `formatOnType` | format with the on-type formatter after its trigger characters (off) |
| `format_on_paste` | `formatOnPaste` | format pasted text with the range formatter (off) |
| `empty_selection_clipboard` | `emptySelectionClipboard` | copying with no selection copies the line |
| `sticky_tab_stops` | `stickyTabStops` | |
| `use_tab_stops` | `useTabStops` | Backspace in indentation deletes to the previous indent stop (on) |
| `trim_whitespace_on_delete` | `trimWhitespaceOnDelete` | deleting a line break also deletes the next line's leading whitespace (off) |
| `multi_cursor_modifier`, `multi_cursor_paste`, `multi_cursor_limit`, `multi_cursor_merge_overlapping` | `multiCursorModifier`, … | Alt or Cmd/Ctrl to add cursors; spread or full paste |
| `hover` (`enabled`, `delay`, `sticky`, `hiding_delay`, `above`, `show_long_line_warning`) | `hover` | when the mouse shows hovers (`On`, `Off`, or `OnKeyboardModifier`: while the modifier opposite `multi_cursor_modifier` is held), after how long, whether it stays while the mouse moves onto it, how long it waits for the mouse, which side it prefers, and whether long lines show a warning |
| `column_selection` | `columnSelection` | mouse drags and Shift+arrows make column selections |
| `overtype_cursor_style`, `overtype_on_paste` | `overtypeCursorStyle`, `overtypeOnPaste` | overtype mode (Insert) |
| `comments` | `comments` | `insert_space`, `ignore_empty_lines` |
| `render_whitespace`, `render_control_characters`, `render_final_newline` | same names | `·` and `→` for whitespace; `␇` for control characters |
| `guides`, `rulers` | `guides`, `rulers` | indent guides (with the active one highlighted: `highlight_active_indentation` is `On`, `Off` or `Always`, which also highlights it on lines with bracket pair guides), bracket pair guides (`bracket_pairs`: off, active or on, plus horizontal ones) and vertical rulers |
| `bracket_pair_colorization` | `bracketPairColorization` | `enabled`: color brackets by nesting level (on); `independent_color_pool_per_bracket_type`: each bracket kind counts its own nesting (off) |
| `occurrences_highlight`, `occurrences_highlight_delay` | `occurrencesHighlight`, `occurrencesHighlightDelay` | `Off`, `SingleFile` (default) or `MultiFile`, which like `SingleFile` highlights in this editor only (editors don't share highlights) |
| `occurrences_highlight_from_all_providers` | `occurrencesHighlightFromAllProviders` | ask every document highlight provider at once and merge their answers (off: the first provider that answers wins, and an empty answer counts); the textual occurrences are only used when no provider is registered |
| `selection_highlight`, `selection_highlight_max_length`, `selection_highlight_multiline` | same names | highlight other occurrences of the selection |
| `selected_text_match_mode` | `selectedTextMatchMode` | how Cmd/Ctrl+D, select all occurrences and selection highlights match selected text: `FindOptions` (the find widget's options while it's open), `CaseSensitive` or `CaseInsensitive` |
| `single_line` | — | a one-line input. Enter, Tab and Escape go to the parent, and pasted line breaks become spaces. |

Model-level settings live in `ModelOptions` (Monaco `ITextModelUpdateOptions`), set with `model.update_options(…)`:

- `tab_size`
- `indent_size`
- `insert_spaces`
- `max_tokenization_line_length` (Monaco `maxTokenizationLineLength`, default 20 000): longer lines aren't highlighted
- `detect_indentation` (Monaco `detectIndentation`, default on): when the model is created, `tab_size`, `indent_size` and `insert_spaces` are guessed from its text, with the given values as the fallback. Pass creation options with `TextModel::new(text).with_options(…)`. `model.detect_indentation(…)` and the "Detect Indentation from Content" command guess again later.
- `trim_auto_whitespace` (Monaco `trimAutoWhitespace`, default on): indentation that Enter or Tab inserted is removed by the next edit if the line is still blank
- `large_file_optimizations` (Monaco `largeFileOptimizations`, default on): a model created with more than 20M characters or 300K lines is never tokenized, and has no folding, sticky scroll, code lenses, occurrence or selection highlights, or wrapping (`TextModel::is_too_large_for_tokenization`). Only the creation options count.
- `default_eol` (Monaco `defaultEOL`, default LF): the line ending of text without line breaks, when the model is created (`with_options`) or its value set

Changing options takes effect on the next frame. Wrapping, folding and the minimap update incrementally.
