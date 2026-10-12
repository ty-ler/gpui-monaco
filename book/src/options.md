# Options

`EditorOptions` mirrors Monaco's `IEditorOptions`. Its defaults are standalone Monaco's: Monaco's defaults with the overrides of its `editor.api.ts` (`glyph_margin` off, two `overview_ruler_lanes`, `wrapping_indent: None` and `auto_indent: Advanced`, where VS Code has on, 3, `Same` and `Full`). To change options, clone the current ones, edit the copy and apply it:

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
| `font_family`, `font_size`, `line_height` | `fontFamily`, `fontSize`, `lineHeight` | `line_height: None` derives it from the font size, as Monaco does; see [Validation](#validation) |
| `font_weight`, `font_ligatures` | `fontWeight`, `fontLigatures` | weights 1–1000; ligatures are `Off` (default), `On`, or CSS `font-feature-settings` |
| `mouse_wheel_zoom` | `mouseWheelZoom` | Cmd+wheel (Ctrl elsewhere) and trackpad pinch zoom every editor's font; see `set_editor_zoom_level` |
| `line_numbers` | `lineNumbers` | `On`, `Off`, `Relative`, `Interval` (every 10th, the current and the last line), or `LineNumbers::custom(\|n\| …)` for Monaco's function: a label per 1-based line, empty for none |
| `select_on_line_numbers` | `selectOnLineNumbers` | clicking a line number selects the line (on); other gutter clicks never select |
| `line_numbers_min_chars`, `line_decorations_width`, `glyph_margin` | same names | gutter layout (see [Layout](#layout)); `glyph_margin` is off by default, as in standalone Monaco; `line_decorations_width` takes pixels (`px(10.).into()`) or `LineDecorationsWidth::Chars(1.5)` for Monaco's `"1.5ch"` |
| `render_line_highlight` | `renderLineHighlight` | `None`, `Gutter`, `Line`, `All` |
| `cursor_style`, `cursor_width`, `cursor_height` | `cursorStyle`, `cursorWidth`, `cursorHeight` | `Line`, `Block`, `Underline`, `LineThin`, `BlockOutline`, `UnderlineThin`; `cursor_width` 0 (the default) draws 2px and it's at most a character wide; a bar of 2px or more straddles the character boundary, 1px to the left. A block cursor (and a bar wider than 2px) draws the character under it in `editorCursor.background` (the caret color's opposite if unset); on a tab it's half a character wide. With several cursors they use `editorMultiCursor.primary.*` and `secondary.*`. `cursor_height` 0 is the line height |
| `mouse_style` | `mouseStyle` | the mouse cursor over the text: `Text` (I-beam), `Default` (arrow) or `Copy` |
| `mouse_middle_click_action` | `mouseMiddleClickAction` | `Default`, `OpenLink` (a middle click opens a link) or `CtrlLeftClick` (it acts like Cmd/Ctrl+click) |
| `cursor_blinking` | `cursorBlinking` | `Blink`, `Smooth`, `Phase`, `Expand`, `Solid` (read-only editors don't blink) |
| `cursor_smooth_caret_animation` | `cursorSmoothCaretAnimation` | `Off`, `Explicit` (keys and clicks, not typing), `On`; every cursor slides |
| `cursor_surrounding_lines`, `cursor_surrounding_lines_style` | same names | lines kept around the cursor; `All` applies them to mouse moves too |
| `smooth_scrolling` | `smoothScrolling` | animate reveals, cursor moves, mouse wheels and the scroll commands (off) |
| `scroll_beyond_last_column`, `reveal_horizontal_right_padding` | same names | room past the longest line (4 columns; none when wrapping at the viewport, which doesn't scroll horizontally) and right of revealed positions (15px) |
| `fast_scroll_sensitivity`, `scroll_predominant_axis` | same names | Alt+wheel speed (5); scroll along the main axis only (on) |
| `scroll_on_middle_click` | `scrollOnMiddleClick` | a middle click scrolls toward the mouse until the next click (off) |
| `inertial_scroll` | `inertialScroll` | wheel and trackpad scrolling keeps going and slows down after the events stop, as Monaco's scrollable element does (off; mostly for Linux touchpads, since macOS and Windows already add momentum) |
| `padding` | `padding` | `top` and `bottom` space |
| `rounded_selection`, `render_line_highlight_only_when_focus` | same names | (on, off). Rounded selections get Monaco's reverse-rounded inner corners; high contrast themes draw them square. With several selections, each is inset 1px at its first and last line |
| `read_only` | `readOnly` | edits are refused and show `read_only_message` |
| `read_only_message` | `readOnlyMessage` | markdown; defaults to "Cannot edit in read-only editor" |
| `placeholder` | `placeholder` | shown while the model is empty |
| `unicode_highlight` | `unicodeHighlight.*` | `non_basic_ascii`, `invisible_characters`, `ambiguous_characters`, `include_comments`, `include_strings`, `allowed_characters`, `allowed_locales` |
| `drag_and_drop`, `double_click_selects_block` | `dragAndDrop`, `doubleClickSelectsBlock` | drag the selection to move it (Alt/Ctrl copies); double-click inside brackets or quotes selects their content |
| `auto_indent_on_paste`, `auto_indent_on_paste_within_string` | `autoIndentOnPaste`, `autoIndentOnPasteWithinString` | re-indent pasted lines (off by default; needs `auto_indent: Full`) |
| `selection_clipboard` | `selectionClipboard` | Linux primary selection: selected text is copied to it, and a middle click pastes it (default on) |
| `stop_rendering_line_after` | `stopRenderingLineAfter` | draw at most this many characters of a line, then "Show more" (default 10 000) |
| `aria_label`, `aria_required`, `accessibility_page_size` | `ariaLabel`, `ariaRequired`, `accessibilityPageSize` | the screen reader name and the page size, in view lines, of what it reads: the pages holding the selection's start and end (500) |
| `accessibility_support` | `accessibilitySupport` | `Auto` (optimized while assistive technology is connected), `On` or `Off`. Optimized, wrapping is measured without continuation indent, and a model mostly made of lines of 10 000+ characters wraps at the viewport (unless `word_wrap_override1` is set); a diff editor stays side by side when narrow. `Off` gives the screen reader no text, only "The editor is not accessible at this time." |
| `screen_reader_announce_inline_suggestion` | `screenReaderAnnounceInlineSuggestion` | screen readers announce new ghost text, or where a next edit suggestion is (on); `CodeEditor::announce` (assertive) and `CodeEditor::announce_status` (polite) announce anything |
| `inline_completions_accessibility_verbose` | `inlineCompletionsAccessibilityVerbose` | adds the accessible view's keybinding to the announcement (off); like standalone Monaco, there's no accessible view, so it adds nothing |
| `tab_focus_mode` | `tabFocusMode` | Tab moves focus instead of indenting; `set_tab_focus_mode` (Ctrl+M) turns it on for every editor |
| `tab_index` | `tabIndex` | the editor's place in the window's Tab order (0); -1 takes it out |
| `stable_peek` | `stablePeek` | Escape doesn't close peek views and double clicks in their preview don't open the result (off) |
| `accessibility_help_url` | `accessibilityHelpUrl` | stored only: there's no accessibility help (Alt+F1) yet |
| `drop_into_editor`, `paste_as` | `dropIntoEditor`, `pasteAs` | `enabled` and when to show the button that switches edits |
| `unusual_line_terminators` | `unusualLineTerminators` | `Prompt` (default), `Auto` or `Off` for LS and PS characters |
| `word_separators` | `wordSeparators` | used by word movement and whole-word find |
| `word_segmenter_locales` | `wordSegmenterLocales` | BCP 47 tags (`ja`, `zh-CN`, `th`, …) whose words word movement, deletion and double-click selection find with ICU4X's word segmenter, for languages written without spaces (empty by default). The first valid tag is used; invalid ones are ignored. Needs the `word-segmenter` cargo feature (on by default) |
| `word_wrap`, `word_wrap_column`, `wrapping_indent` | `wordWrap`, `wordWrapColumn`, `wrappingIndent` | `Off`, `On`, `WordWrapColumn`, `Bounded`; continuation lines are indented `None` (default, and they have no indent guides), `Same`, `Indent` or `DeepIndent` |
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
| `scrollbar` | `scrollbar.*` | `vertical`/`horizontal` (`Auto` shows while hovered or just scrolled, `Visible`, `Hidden`), `use_shadows`, `*_has_arrows`, `arrow_size`, `*_slider_size`, `handle_mouse_wheel`, `always_consume_mouse_wheel`, `scroll_by_page` (a track click pages by the viewport's height, then drags the slider), `ignore_horizontal_scrollbar_in_content_height` (without `scroll_beyond_last_line`, a shown horizontal scrollbar adds its height to the content height so the last line can scroll above it; this leaves it out) |
| `overview_ruler_lanes`, `overview_ruler_border`, `hide_cursor_in_overview_ruler` | same names | lanes (2, at most 3; 0 hides the marks), a left border, the cursor marks |
| `minimap` | `minimap` | `enabled`, `max_column`, `scale`, `always_show_slider`, `side` (right or left), `render_characters`, `size` (`Proportional`, `Fill`, `Fit`), section headers (`show_region_section_headers`, `show_mark_section_headers`, `mark_section_header_regex`, `section_header_font_size`, `section_header_letter_spacing`), `autohide` (`None`; `Mouseover` shows it while hovered; `Scroll` shows it while scrolling and lets clicks through while hidden) |
| `sticky_scroll` | `stickyScroll` | `enabled`, `max_line_count`, `default_model`, `scroll_with_editor` |
| `auto_closing_brackets`, `auto_closing_quotes`, `auto_closing_comments` | `autoClosingBrackets`, … | `Always`, `LanguageDefined`, `BeforeWhitespace`, `Never` |
| `auto_closing_overtype`, `auto_closing_delete`, `auto_surround` | same names | |
| `auto_indent` | `autoIndent` | `None`, `Keep`, `Brackets`, `Advanced` (default: brackets and onEnter rules), `Full` (also the language's indentation rules) |
| `match_brackets` | `matchBrackets` | `Always` (the pair at the cursor, or else the enclosing pair), `Near` or `Never` |
| `code_lens`, `code_lens_font_family`, `code_lens_font_size` | `codeLens`, … | show code lenses (on); their font (the editor's; 90% of its size) |
| `linked_editing` | `linkedEditing` | mirror edits between linked ranges, like HTML tag pairs (off) |
| `inline_suggest` | `inlineSuggest.*` | `enabled` (on), `mode` (`SubwordSmart`), `suppress_suggestions`, `keep_on_blur`, `min_show_delay`, `suppress_in_snippet_mode` (on), `show_toolbar` (`OnHover`, `Always`, `Never`), `syntax_highlighting_enabled` (on), `font_family` (the editor's), `edits` (inline edits: `enabled` (on), `show_collapsed` (off), `render_side_by_side` (`Auto`, `Never`), `allow_code_shifting` (`Always`, `Horizontal`, `Never`), `show_long_distance_hint` (on), `long_distance_hint_context_line_count` (0, up to 10)), `trigger_command_on_provider_change` (off: a provider's refresh asks every provider for inline edits instead), `experimental` (`suppress_inline_suggestions`: provider groups not asked; `show_on_suggest_conflict`: `Never`, `Always` or `WhenSuggestListIsIncomplete`) |
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
| `suggest` | `suggest.*` | `insert_mode`, `filter_graceful`, `match_on_word_start_only`, `show_deprecated`, `hidden_kinds`, `locality_bonus`, `font_size`, `line_height`, `show_icons`, `show_status_bar`, `show_inline_details`, `preview`, `preview_mode`, `selection_mode` (whether a suggestion is focused when the list opens by itself: `Always`, `Never`, `WhenTriggerCharacter` or `WhenQuickSuggestion`), `snippets_prevent_quick_suggestions` (off), `share_suggest_selections` (selection memories shared by every window, off), `fit_width_to_details` (off) |
| `tab_completion` | `tabCompletion` | `Off`, `On` (Tab inserts the best suggestion and cycles through the others) or `OnlySnippets` (like `Off` until snippets land) |
| `inlay_hints` | `inlayHints` | whether inlay hints show (`On`, `OnUnlessPressed`, `OffUnlessPressed`, `Off`), padding, the per-line maximum length, and `font_size` (0, under 5 or above the editor's uses the editor's) and `font_family` |
| `parameter_hints` | `parameterHints` | `enabled`: open parameter hints while typing (on); `cycle`: going past the last or first signature wraps around instead of closing them (on) |
| `folding_strategy` | `foldingStrategy` | `Auto` (providers, then syntax, then indentation) or `Indentation` |
| `folding_maximum_regions` | `foldingMaximumRegions` | at most this many provider or syntax fold ranges (5000) |
| `folding_highlight` | `foldingHighlight` | shade the first line of collapsed regions (on), over selections and indent guides, as a whole-line decoration |
| `unfold_on_click_after_end_of_line` | `unfoldOnClickAfterEndOfLine` | unfold when clicking after a collapsed line (off) |
| `folding_imports_by_default` | `foldingImportsByDefault` | collapse import ranges at first (off) |
| `smart_select` | `smartSelect` | expand selection over leading/trailing whitespace lines and word parts (both on) |
| `lightbulb` | `lightbulb.enabled` | when the code action lightbulb asks for actions: `Off` (never shown), `OnCode` (default) or `On` |
| `code_action_widget` | `editor.codeActionWidget.*` | `show_headers`: group the code action menu under headers (on) |
| `goto_location` | `gotoLocation` | what go to definition, references and the other navigation commands do with several results, and their alternative commands. `multiple_tests` and `alternative_tests_command` are for a host's go to test commands. Monaco's deprecated `multiple` is `GotoLocationOptions::with_multiple`, which sets every `multiple_*` |
| `peek_widget_default_focus` | `peekWidgetDefaultFocus` | what a new peek view focuses: `Tree` (the results list, default) or `Editor` (the preview) |
| `definition_link_opens_in_peek` | `definitionLinkOpensInPeek` | Cmd/Ctrl+click on a symbol shows its definitions in a peek view (off) |
| `find` | `find.*` | `cursor_move_on_type` (on), `seed_search_string_from_selection`, `auto_find_in_selection`, `add_extra_space_on_top` (on), `loop_` (on), `history` and `replace_history` (on), `close_on_result` (off), `find_on_type` (search while typing; when off, Enter searches; on), `global_find_clipboard` (share the search string through the find pasteboard on macOS, or between the app's editors; off) |
| `format_on_type` | `formatOnType` | format with the on-type formatter after its trigger characters (off) |
| `format_on_paste` | `formatOnPaste` | format pasted text with the range formatter (off) |
| `empty_selection_clipboard` | `emptySelectionClipboard` | copying with no selection copies the line |
| `sticky_tab_stops` | `stickyTabStops` | Left and Right move over indentation spaces a tab stop at a time, and clicks in such indentation land on the nearest tab stop |
| `use_tab_stops` | `useTabStops` | Backspace in indentation deletes to the previous indent stop (on) |
| `trim_whitespace_on_delete` | `trimWhitespaceOnDelete` | deleting a line break also deletes the next line's leading whitespace (off) |
| `multi_cursor_modifier`, `multi_cursor_paste`, `multi_cursor_limit`, `multi_cursor_merge_overlapping` | `multiCursorModifier`, … | Alt or Cmd/Ctrl to add cursors; spread or full paste |
| `hover` (`enabled`, `delay`, `sticky`, `hiding_delay`, `above`, `show_long_line_warning`) | `hover` | when the mouse shows hovers (`On`, `Off`, or `OnKeyboardModifier`: while the modifier opposite `multi_cursor_modifier` is held), after how long, whether it stays while the mouse moves onto it, how long it waits for the mouse, which side it prefers, and whether long lines show a warning |
| `column_selection` | `columnSelection` | mouse drags and Shift+arrows make column selections |
| `overtype_cursor_style`, `overtype_on_paste` | `overtypeCursorStyle`, `overtypeOnPaste` | overtype mode (Insert) |
| `comments` | `comments` | `insert_space`, `ignore_empty_lines` |
| `render_whitespace`, `render_control_characters`, `render_final_newline` | same names | whitespace as Monaco's default SVG dots and arrows (`boundary` skips single spaces between non-spaces; a tab doesn't count as a space); control characters in white on dark red, C0 controls and DEL as `␇` and the like, and bidi formatting characters (U+202A–202E, U+2066–2069, U+200E/F, U+061C) as `[U+202E]`, so they can't reorder the line; the BOM and line/paragraph separators always show as U+FFFD; the final newline's line number is `Dimmed` by default on Linux and `On` elsewhere, as in Monaco |
| `guides`, `rulers` | `guides`, `rulers` | indent guides (with the active one highlighted: `highlight_active_indentation` is `On`, `Off` or `Always`, which also highlights it on lines with bracket pair guides), bracket pair guides (`bracket_pairs`: off, active or on, plus horizontal ones) and vertical rulers |
| `bracket_pair_colorization` | `bracketPairColorization` | `enabled`: color brackets by nesting level (on); `independent_color_pool_per_bracket_type`: each bracket kind counts its own nesting (off) |
| `occurrences_highlight`, `occurrences_highlight_delay` | `occurrencesHighlight`, `occurrencesHighlightDelay` | `Off`, `SingleFile` (default) or `MultiFile`. With `MultiFile` and another editor showing a different model, the word's textual occurrences are highlighted in every model (standalone Monaco's only multi-document highlight provider; the document highlight providers aren't asked), and each editor shows its model's. Highlights are computed 50ms after the cursor rests and shown no sooner than the delay (0) after it moved |
| `occurrences_highlight_from_all_providers` | `occurrencesHighlightFromAllProviders` | ask every document highlight provider at once and merge their answers (off: the first provider that answers wins, and an empty answer counts); the textual occurrences are only used when no provider is registered |
| `selection_highlight`, `selection_highlight_max_length`, `selection_highlight_multiline` | same names | highlight other occurrences of the selection |
| `selected_text_match_mode` | `selectedTextMatchMode` | how Cmd/Ctrl+D, select all occurrences and selection highlights match selected text: `FindOptions` (the find state's match case and whole word, open or closed; both off by default), `CaseSensitive` or `CaseInsensitive` |
| `single_line` | — | a one-line input. Enter, Tab and Escape go to the parent, and pasted line breaks become spaces. |

Model-level settings live in `ModelOptions` (Monaco `ITextModelUpdateOptions`), set with `model.update_options(…)`:

- `tab_size` (at least 1)
- `indent_size` (at least 1)
- `indent_size_is_tab_size` (Monaco `indentSize: 'tabSize'`, off by default): `indent_size` follows `tab_size`. Models created with `detect_indentation` start with it on, as Monaco's do.
- `insert_spaces`
- `max_tokenization_line_length` (Monaco `maxTokenizationLineLength`, default 20 000): longer lines aren't highlighted
- `detect_indentation` (Monaco `detectIndentation`, default on): when the model is created, `tab_size`, `indent_size` and `insert_spaces` are guessed from its text, with the given values as the fallback. Pass creation options with `TextModel::new(text).with_options(…)`. `model.detect_indentation(…)` and the "Detect Indentation from Content" command guess again later.
- `trim_auto_whitespace` (Monaco `trimAutoWhitespace`, default on): indentation that Enter or Tab inserted is removed by the next edit if the line is still blank
- `large_file_optimizations` (Monaco `largeFileOptimizations`, default on): a model created with more than 20M characters or 300K lines is never tokenized, and has no folding, sticky scroll, code lenses, occurrence or selection highlights, or wrapping (`TextModel::is_too_large_for_tokenization`). Only the creation options count.
- `default_eol` (Monaco `defaultEOL`, default CRLF on Windows and LF elsewhere): the line ending of text without line breaks, when the model is created (`with_options`) or its value set

Changing options takes effect on the next frame. Wrapping, folding and the minimap update incrementally.

## Validation

Like Monaco's `validate` methods, the editor clamps options to their limits and gives Monaco's sentinel values their meaning: it renders with `options.validated()`, while `editor.options()` returns the options as they were set (Monaco's `getRawOptions`). `editor.computed_options()` returns what it runs with (Monaco's `getOptions` and `getOption`): validated, zoomed, with `font_size` and `line_height` in pixels, and `accessibility_support` resolved to `On` or `Off`. `ConfigurationChange` carries those. `layout_info()` has the rest of Monaco's computed layout, such as `lineDecorationsWidth` in pixels. Whole-number options are truncated, as Monaco's `clampedInt` does. In particular:

- `font_size` is 6–100px, and 0 means the platform default (`resolved_font_size`).
- `line_height` is clamped to 0–150. `None` or 0 computes it from the font size (1.5× on macOS, 1.35× elsewhere), a value below 8 multiplies the font size (`Some(px(1.5))` is 1.5 lines), and the result is rounded and at least 8px (`resolved_line_height`).
- `fast_scroll_sensitivity` of 0 or less is 5; `mouse_wheel_scroll_sensitivity` of 0 is 1.
- `line_numbers_min_chars` is 1–300; `line_decorations_width`, `padding`, `reveal_horizontal_right_padding` and the scrollbar sizes 0–1000px; `overview_ruler_lanes` 0–3; `word_wrap_column`, `multi_cursor_limit` (to 100 000) and `accessibility_page_size` at least 1; `folding_maximum_regions` 10–65 000; `color_decorators_limit` 1–1 000 000; `code_lens_font_size` and `inlay_hints.font_size` 0–100; `suggest.font_size` and `suggest.line_height` 0–1000; ruler columns at most 10 000.
- `minimap.scale` is 1, 2 or 3; `minimap.max_column` 1–10 000; `minimap.section_header_font_size` 4–32; `sticky_scroll.max_line_count` 1–20. A `minimap.mark_section_header_regex` that doesn't compile keeps the default.
- `occurrences_highlight_delay` is at most 2s, `hover.delay` 10s, `hover.hiding_delay` 10 minutes and `inline_suggest.min_show_delay` 10s.

`ModelOptions::validated` (applied by `TextModel`) keeps `tab_size` and `indent_size` at least 1.

## Layout

The gutter and minimap widths follow Monaco's `EditorLayoutInfoComputer.computeLayout`, and `editor.layout_info()` reports them:

- the glyph margin is a line height wide;
- the line numbers are the widest digit times the last line number's digits, at least `line_numbers_min_chars`;
- the line decorations are `line_decorations_width` (pixels, or `LineDecorationsWidth::Chars(n)` for Monaco's `"Nch"`), plus 16px for the folding controls unless `show_folding_controls` is `Never`;
- the minimap takes the share of the rest that shows about the columns the text area does (its characters are `scale` pixels wide, plus an 8px gutter), at most `max_column` characters. `minimap.size: Fill` and `Fit` scale it to the editor's height, drawing larger scales into the same width.

## Global options

Some of Monaco's `IGlobalEditorOptions` apply to every editor, so they're free functions:

- `autoDetectHighContrast` is `set_auto_detect_high_contrast` (on by default). Monaco reads the OS's high contrast mode from the browser; GPUI doesn't report it, so the host calls `set_os_high_contrast(cx, true)` when the OS is in high contrast mode (for example from the platform's accessibility settings). Editors then switch to `hc-black` or `hc-light`, keeping dark or light, and back to `vs-dark` or `vs` when it's off.
- `tabFocusMode` is also `set_tab_focus_mode`.

## Differences in shape

Some options are shaped differently from Monaco's:

- `suggest.show*` (27 booleans) are `SuggestOptions::hidden_kinds`, a list of the `CompletionItemKind`s to hide. Monaco's `showColors: false` hides both `Color` and `Customcolor`, so list both.
- Monaco's top-level `suggestFontSize` and `suggestLineHeight` are `suggest.font_size` and `suggest.line_height`.
- `scrollbar.verticalScrollbarSize` and `scrollbar.horizontalScrollbarSize` are the top-level `vertical_scrollbar_size` and `horizontal_scrollbar_size`.
- The model-level `bracketPairColorizationOptions` is `EditorOptions::bracket_pair_colorization`.
- `lineNumbers` takes `LineNumbers::custom(…)` for Monaco's function form, and `lineDecorationsWidth` `LineDecorationsWidth::Chars` for its `"Nch"` form.
- `gotoLocation.multiple` (deprecated) is `GotoLocationOptions::with_multiple`.
- Sizes are `Pixels` and delays `Duration`s.

These Monaco options have no field because they'd change nothing here: `renameOnType` (a deprecated alias of `linked_editing`), `inDiffEditor` (an internal context key), `disableMonospaceOptimizations`, `renderRichScreenReaderContent`, `experimentalWhitespaceRendering`, `experimentalGpuAcceleration` and `allowOverflow` (DOM rendering choices), and `allowVariableFonts` and `allowVariableFontsInAccessibilityMode` (only injected text carries its own font here). `letterSpacing` and `fontVariations` wait on GPUI text support (#232, #233).
