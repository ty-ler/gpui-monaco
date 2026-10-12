# Differences from Monaco

gpui-monaco aims to *behave* like Monaco. Where it differs, it's usually because of Rust, GPUI or the MVP scope.

## API shape

- **Positions:** 0-based `PointUtf16 { row, column }` instead of 1-based `{ lineNumber, column }`. Use `PointUtf16::from_monaco` and `to_monaco` to convert. View zones' `after_line` keeps Monaco's 1-based meaning.
- **Edits and decorations:** take byte offset ranges, not `IRange`.
- **Options:** a typed `EditorOptions` struct, not a dynamic options bag. Change them with `update_options`.
- **Commands:** keybindings and commands are GPUI actions. Monaco's `addAction` is `add_action`, and the global `addCommand`, `addEditorAction` and `addKeybindingRules` are free functions returning a `Disposable`; keybindings use GPUI keystroke syntax and key context predicates instead of key codes and `when` clauses. See [Keybindings](keymap.md).
- **Global events:** Monaco's `monaco.editor.onDid…` events take a listener and return a gpui `Subscription`. Creation events arrive after the current update, and `on_will_dispose_model` gets the model's id and state, since a released entity can't be handed out.
- **Editor events and geometry:** `CodeEditorEvent`s are notifications. Listeners can't add clipboard data in `WillCopy` or stop a key in `KeyDown`. Layout queries (`layout_info`, `top_for_line_number`, `content_width`, …) read the last frame, so they return `None` before the first paint, and `LayoutChanged` arrives after the frame that changed the layout.
- **Providers:** registered app-wide per language selector. A provider returns a GPUI `Task`, not a `Promise`, and has no cancellation token; dropping the task cancels it.
- **Highlighting:** done by tree-sitter grammars and queries, or by Monarch grammars given as JSON (`set_monarch_tokens_provider`). Monaco theme rules apply to both: captures are mapped to Monaco token names and TextMate scopes, and Monarch tokens are Monaco token names.

## Behavior

The repository's [`docs/MONACO_PARITY.md`](https://github.com/ty-ler/gpui-monaco/blob/main/docs/MONACO_PARITY.md) has a full feature-by-feature audit with tracking issues. The highlights:


- **Find regexes:** use the Rust [`regex`](https://docs.rs/regex) crate (linear time). It has no lookahead, lookbehind or backreferences.
  - `\n` in a pattern still enables multiline search, as in Monaco.
  - Replacement patterns support `$1`, `$&`, `$$`, `\n`, `\t` and Monaco's case-changing `\u`, `\U`, `\l` and `\L`. Case changes work on whole characters rather than UTF-16 units, so letters outside the Basic Multilingual Plane change case too.
  - Language configuration patterns (`wordPattern`, `indentationRules`, …) use `fancy-regex`, so VS Code's configurations work unchanged.
- **Cursor and scrolling:** Monaco's blinking styles are CSS animations on the compositor; here the editor repaints, but only while the caret's opacity or height is changing. `inertial_scroll` follows Monaco's `scrollableElement.ts` (not vendored here): each wheel event sets a speed that's applied every 1/60s and decays by a tenth, on top of any momentum the platform adds.
- **Context menu:** it's drawn by the editor, in the window, flipping up or left at the window's edges, rather than as an OS-level menu. It's as wide as its widest row (130px at least), and submenus open after Monaco's 250ms delay on the side with room. The scrollbar menu and the "Copy As" and "Share" submenus aren't there. Sticky scroll lines have Monaco's own menu (Toggle Editor Sticky Scroll).
- **Keybinding labels:** menus, the command palette and titles label keys as Monaco's `ResolvedKeybinding.getLabel()` does ("F12", "⇧⌥F", "Ctrl+Shift+P"; GPUI's `fn` modifier isn't shown). `keybinding_label` has the formatter for your own UI.
- **Overflowing widgets:** widgets with `allow_editor_overflow` (the hover, suggest, parameter hints, menus, messages, rename) are GPUI deferred draws above the whole window, much like Monaco's `fixedOverflowWidgets`, and they take the mouse from whatever is under them, so the editor's own mouse events don't fire over them.
- **Bracket pair guides:** pairs come from the colorized bracket pairs.
- **Sort Lines:** Monaco's `Intl.Collator` follows the browser's locale; here lines are compared with the Unicode root collation (ICU4X's), which is what English and most locales use. Locale tailorings, such as Swedish putting `ö` after `z`, aren't applied.
- **View lines:** whitespace is always drawn as Monaco's default SVG marks (there's no `experimentalWhitespaceRendering`). The "Show more" chip's 1px border is drawn only in themes with a `contrastBorder` color, but always takes its room.
- **Advanced wrapping:** Monaco lets the browser break lines in a hidden DOM node. Here, characters are measured in the editor font and breaks follow the same break-before/after characters as `Simple` wrapping. `disableMonospaceOptimizations` isn't needed: cursor and selection positions always come from shaped text.
- **Unicode highlighting:** there's no workspace trust, so Monaco's `inUntrustedWorkspace` defaults resolve as trusted, as in the standalone editor. When there are too many highlights, nothing is highlighted and a banner above the text offers to turn the most common kind off, as in Monaco. "Adjust settings", the banner and the `disableHighlightingOf*` commands change the editor's options, not user settings.
- **Unusual line terminators:** Monaco asks in a modal dialog; here a quick pick asks, and dismissing it ignores them, like the dialog's Ignore.
- **Colorize:** `colorize` returns lines and highlights (and `colorize_element` an element) rather than HTML. For tree-sitter languages, token types come from captures mapped to Monaco's names, so they can differ from a Monarch tokenizer's; Monarch languages give their own.
- **Monarch regexes:** JavaScript regexes run on `fancy-regex`, translated to keep JavaScript's `\d`, `\w`, `\s` and `.`. `\b` uses Unicode word characters, matching is by code point rather than UTF-16 code unit, and octal escapes aren't supported. Only Monarch languages can be embedded with `nextEmbedded`; others get a `""` token per line.
- **Copy with syntax highlighting:** not available, because GPUI's clipboard holds plain text, images and files but no HTML or RTF.
- **Drop and paste:** GPUI delivers dropped files, not dragged text from other apps, and the clipboard's text only (no `text/html` or images), so the HTML provider only applies to data given to `drop_data`/providers directly. There's no relative path provider, because the editor has no workspace. Data prepared at copy time (`prepareDocumentPaste`) is kept in memory for the latest copy, as in Monaco, so it reaches pastes in the same app only. The options button's picker is the quick pick, not Monaco's action list, and the progress spinner for slow providers is drawn rather than a spinning codicon, as GPUI can't rotate text.
- **Fonts:** `letterSpacing` and `fontVariations` aren't supported, because GPUI text has neither. Decorations change the font's family, weight and style (`fontFamily`, `fontWeight`, `fontStyle`), but not its size (`fontSize`), because GPUI shapes a line in one font size; wrapping measures the editor's font. Taller lines from `line_height` stay uniform in the minimap and sticky scroll. A variable font's weight comes from `font_weight`.
- **Minimap section headers:** `section_header_letter_spacing` is accepted but not applied, because GPUI text has no letter spacing.
- **Minimap glyphs at `scale` 2 and 3 on Retina displays:** Monaco draws its built-in glyphs at scales 1 and 2 (in device pixels) and samples the editor font for larger ones. GPUI can't rasterize a font into a buffer, so larger scales enlarge the scale-2 glyphs.
- **Decoration colors:** overview ruler and minimap colors are a resolved color or a theme color id (`DecorationColor`); markers take the theme's `minimap.{error,warning,info}Highlight` and overview ruler colors.
- **Injected text:** as in Monaco, inlay hints, `before`/`after` text and ghost text are in the view lines' columns: selections are drawn and hit-tested across them, wrapping counts them (and may break inside one), and Up/Down keep the visible column with them included. Where both show at one column, `after` text comes before `before` text (Monaco has the reverse). `widthInEm` isn't supported, and `Advanced` wrapping measures injected text in the editor's font, not its own.
- **Color picker:** the standalone picker isn't bound to a key (as in Monaco). It runs from the command palette or `ShowOrFocusStandaloneColorPicker`.
- **Quick access:**
  - Symbols are matched with the suggest widget's fuzzy scorer, not VS Code's quick open scorer.
  - The command palette has no similar-command or TF-IDF suggestions (those are VS Code workbench features), and accents aren't folded when matching.
- **Inline completions:**
  - `completeBracketPairs` tokenizes the completed line on its own, so a completion inside a multi-line comment or string isn't recognized as such.
  - Inline edits draw their boxes as content widgets. Their transitions are ported but for the gutter icon's turn towards the edit (the icon is a glyph, which GPUI can't rotate; the arrow points the right way at once), the pill's transition being suspended while Shift is held, and the onboarding pulses.
  - `userPrompt`, the model picker (`modelInfo`) and provider options aren't ported, nor the `emptyResponseInformation` telemetry option. The editor asks every provider to the end, so lists are never disposed as `LostRace`.
- **Snippets:**
  - Transform regexes run on `fancy-regex`, translated from JavaScript, so exotic JavaScript syntax may not compile.
  - `/upcase` and `/downcase` ignore the locale (Monaco uses `toLocaleUpperCase`).
  - `$CURRENT_*` day and month names are English.
- **Word-based suggestions:** the documents are the models registered with `register_model`.
- **Suggestion memories:** they are kept in memory for the app's lifetime, not persisted, per window (Monaco's workspace storage) or, with `suggest.share_suggest_selections`, for every window (its profile storage). The same goes for the suggest widget's size and whether details are expanded.
- **Suggest widget:** the details panel's text can't be selected, because GPUI has no selectable text, so copying from it (`suggestWidgetCopy`) copies the whole detail and documentation.
- **Code actions, rename, inlay hints:** there's no Refactor Preview panel: previews go to your `set_workspace_edit_preview_handler`, and apply directly without one, as in standalone Monaco. There's no `editor.rename.enablePreview` setting; previews are offered whenever a handler is set. Right-clicking injected text without a location doesn't open the editor's menu, as in Monaco. [`docs/MONACO_AUDIT.md`](https://github.com/ty-ler/gpui-monaco/blob/main/docs/MONACO_AUDIT.md) lists every known gap.
- **Folding:** with `folding_strategy: Auto`, ranges come from folding range providers, else the grammar's tree-sitter fold query (`folds.scm`), else indentation and markers. Monaco has no syntax folding, so it goes straight to indentation.
- **Accessibility:** screen readers get the lines around the selection as AccessKit text runs, as Monaco's hidden textarea does. Monaco's `alert` (tab focus mode, inline suggestions, `CodeEditor::announce`) is an assertive AccessKit live region under the editor. `accessibility_support: Auto` follows whether assistive technology is connected to the window. The inline suggestion sound (`playSignal`) and the screen reader text in the inline suggestion hover aren't there, and Off gives no text runs (Monaco on macOS still writes the word before the cursor, for long-press input).
- **Two-cell full-width characters:** glyphs are moved after shaping, so backgrounds of a centered character start at the glyph rather than its cell. Ghost text and the diff editor's deleted lines keep the font's width. `Simple` wrapping already counts full-width characters as two columns with either setting (Monaco uses the font's ratio for `Font`).
- **Diff editor:** `createViewModel` and the diff editor's own `createContextKey` aren't ported (the sides' editors have `create_context_key`). Nor are staying inline through a smooth resize that started below the inline breakpoint, or the animated realignment when comparing a moved block.
- **Long lines:** as in Monaco, very long lines stop rendering after `stop_rendering_line_after` with a "Show more (…)" chip, and clicking at or past that column (Monaco's `longLinesHelper`) sets the option to `None`, so everything draws.
- **Developer actions:** "Developer: Inspect Tokens" shows tree-sitter tokens: the token type is the Monaco-style name `tokenize` gives (`keyword.rust`), followed by the capture and the TextMate scopes it's matched against. Escape closes it on key down rather than key up. "Developer: Force Retokenize" reparses the whole model on the spot and logs how long it took.
- **Icons:** the codicon font is optional (the `codicons` feature, CC-BY-4.0); without it the widgets draw Unicode stand-ins for Monaco's codicons.

## Platform

- **macOS:** the primary platform.
- **Linux and Windows:** build in CI, but get less testing.
- **Keybindings:** follow Monaco's per-platform defaults (Cmd on macOS, Ctrl elsewhere).
