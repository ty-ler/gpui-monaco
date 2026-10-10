# Migrating from Monaco

This page maps Monaco's API to gpui-monaco for people porting a Monaco integration. The shapes follow Rust and GPUI:

- **Models and editors are entities.** Create them with `cx.new`, and read or change them through `read`/`update`.
- **Positions are 0-based.** They're `PointUtf16 { row, column }` with UTF-16 columns, where Monaco uses 1-based `{ lineNumber, column }`. `PointUtf16::from_monaco` and `to_monaco` convert.
- **Events are GPUI events.** Subscribe with `cx.subscribe(&editor, …)` instead of `onDid…` listeners.
- **Async results are `Task`s.** Dropping a task cancels it, so there are no cancellation tokens.

[Differences from Monaco](differences.md) lists behavior that differs.

## Creating editors and models

| Monaco | gpui-monaco |
|---|---|
| `monaco.editor.create(el, { value, language })` | `cx.new(\|cx\| TextModel::new(value).with_language(language))`, then `CodeEditor::new(model, window, cx)` |
| `monaco.editor.createModel(value, language, uri)` | `TextModel::new(value).with_language(language).with_uri(uri)`, then `register_model` so providers can find it |
| `monaco.editor.getModel(uri)` | `model_for_uri(cx, uri)` |
| `monaco.editor.setModelLanguage(model, id)` | `model.set_language(id, cx)` |
| `monaco.editor.setModelMarkers(model, owner, markers)` | `model.set_markers(owner, markers, cx)` |
| `monaco.editor.defineTheme(name, data)` | `Theme::define(name, &base, inherit, rules, &colors)`, or `Theme::from_vscode_json` |
| `monaco.editor.setTheme(name)` | `editor.set_theme(Arc::new(theme), cx)` (per editor) |
| `monaco.editor.colorize(text, lang, opts)` | `colorize::colorize(text, lang, &theme, &registry, opts)` or `colorize_element` |
| `monaco.editor.tokenize(text, lang)` | `colorize::tokenize(text, lang, &registry)` |
| `monaco.languages.register(...)` | `set_language_registry(cx, registry)`, with grammars from `gpui-monaco-languages` |
| `editor.dispose()` / `model.dispose()` | drop the entity |
| `editor.layout()` | not needed: the editor lays itself out every frame |

## Text model

| Monaco `ITextModel` | `TextModel` |
|---|---|
| `getValue()` / `setValue(v)` | `text()` / `set_value(v, cx)` |
| `getLineCount()`, `getLineContent(n)`, `getLineLength(n)` | `line_count()`, `line_content(row)`, `line_len_utf16(row)` |
| `getOffsetAt(pos)` / `getPositionAt(offset)` | `offset_at(point)` / `position_at(offset)` (byte offsets) |
| `getValueInRange(range)` | `value_in_range(range)` |
| `validatePosition`, `validateRange` | `validate_position`, `validate_range` |
| `getWordAtPosition`, `getWordUntilPosition` | `word_at_position`, `word_until_position` |
| `findMatches(...)` | `find_matches(query, is_regex, match_case, whole_word, …)` |
| `applyEdits(edits)` / `pushEditOperations(...)` | `edit(edits, cx)`, grouped with `start_transaction`/`end_transaction` |
| `pushStackElement()` | `push_stack_element()` |
| `undo()` / `redo()`, `canUndo()` / `canRedo()` | `undo(source, cx)` / `redo(source, cx)`, `can_undo()` / `can_redo()` |
| `getEOL()` / `setEOL(...)` | `line_ending()` / `set_line_ending(...)` |
| `getOptions()` / `updateOptions(...)` | `options()` / `update_options(...)` (tab size, insert spaces, …) |
| `deltaDecorations(old, new)` | `delta_decorations(owner, &old, new, cx)` (byte ranges) |
| `getDecorationsInRange(range)` | `decorations_in_range(range, editor)` |
| `onDidChangeContent` | `TextModelEvent::Edited { changes, version, … }` |
| `onDidChangeLanguage`, `onDidChangeOptions`, `onDidChangeDecorations` | `LanguageChanged`, `OptionsChanged`, `DecorationsChanged` |

## Editor

| Monaco `ICodeEditor` | `CodeEditor` |
|---|---|
| `getModel()` / `setModel(m)` | `model()` / `set_model(m, cx)` |
| `getValue()` / `setValue(v)` | through the model: `model().read(cx).text()`, `set_value` |
| `getOptions()` / `updateOptions(o)` | `options()` / `update_options(full_options, cx)`: clone, change, set |
| `getPosition()` / `setPosition(p)` | `selections()[0].head` / `set_selections(&[Selection::collapsed(p)], cx)` |
| `getSelections()` / `setSelections(s)` | `selections()` / `set_selections(&s, cx)` |
| `revealLine`, `revealLineInCenter`, `revealRange…`, `revealPosition…` | `reveal_line`, `reveal_line_in_center`, `reveal_range…`, `reveal_position…` |
| `getScrollTop()` / `setScrollTop(y)` | `scroll_position()` / `set_scroll_position(p, cx)` |
| `getVisibleRanges()` | `visible_ranges(cx)` |
| `getScrolledVisiblePosition(p)` | `scrolled_visible_position(p)` |
| `getTargetAtClientPoint(x, y)` | `mouse_target(point, window, cx)` |
| `focus()` / `hasTextFocus()` | `window.focus(&editor.focus_handle(cx), cx)` / `has_text_focus(window)` |
| `saveViewState()` / `restoreViewState(s)` | `save_view_state()` / `restore_view_state(&s, cx)` |
| `executeEdits(source, edits, endCursorState)` | `execute_edits(edits, end_selections, cx)` |
| `pushUndoStop()` | `push_undo_stop(cx)` |
| `trigger(source, handlerId, payload)` | `trigger(id, window, cx)`, taking a Monaco action id or a GPUI action name |
| `getAction(id).run()` | `run_action(id, window, cx)` |
| `addAction(descriptor)` | `add_action(EditorActionDescriptor::new(id, label, run).keybinding(…), cx)` |
| `addCommand(keybinding, handler)` | define a GPUI action and bind it in `actions::KEY_CONTEXT` |
| `getSupportedActions()` | `supported_actions(cx)` |
| `createDecorationsCollection(d)` | `create_decorations_collection(d, cx)` |
| `deltaDecorations(old, new)` | the model's `delta_decorations(Some(editor_id), …)` (editor-owned) |
| `setHiddenAreas(ranges)` | `set_hidden_areas(rows, cx)` (0-based rows, end exclusive) |
| `changeViewZones(a => a.addZone(z))` | `add_view_zone(zone, cx)`, `set_view_zone_height`, `remove_widget` |
| `addContentWidget(w)` / `layoutContentWidget(w)` | `add_content_widget(w, cx)` / `set_content_widget_position(…)` |
| `addOverlayWidget(w)` | `add_overlay_widget(w, cx)` |
| `addGlyphMarginWidget(w)` | `add_glyph_margin_widget(GlyphMarginWidget { row, view }, cx)` |
| `getContribution('editor.contrib.findController')` | the find methods on `CodeEditor` (`start_find`, …) |
| `getFoldingModel()…` | `folding_ranges(cx)`, `collapsed_folds(cx)` |

### Editor events

| Monaco | `CodeEditorEvent` |
|---|---|
| `onDidChangeCursorSelection` / `onDidChangeCursorPosition` | `SelectionsChanged(SelectionChange { reason, source })` |
| `onDidScrollChange` | `ScrollChanged` |
| `onDidFocusEditorText` / `onDidBlurEditorText` | `Focused` / `Blurred` |
| `onMouseDown`, `onMouseUp`, `onMouseMove`, `onMouseLeave`, `onContextMenu` | `MouseDown`, `MouseUp`, `MouseMove`, `MouseLeave`, `ContextMenu` (with `EditorMouseEvent`) |
| `onDidAttemptReadOnlyEdit` | `ReadOnlyEditAttempt` |
| `onDidChangeModelContent` | subscribe to the model's `TextModelEvent::Edited` |
| opener service / `registerEditorOpener` | `OpenLocation(location)` and `OpenLink(url)`: the host opens them |
| `commandService.executeCommand` from providers | `ExecuteCommand(command)`, or `register_command` |

## Language features

Monaco's `monaco.languages.register…Provider(selector, provider)` functions become `providers::register_…_provider(cx, LanguageSelector, provider)`, where each provider is a Rust trait. `providers::unregister_provider(cx, registration)` replaces `IDisposable.dispose()`.

| Monaco | gpui-monaco trait |
|---|---|
| `registerCompletionItemProvider` | `CompletionProvider` |
| `registerHoverProvider` | `HoverProvider` |
| `registerSignatureHelpProvider` | `SignatureHelpProvider` |
| `registerDefinitionProvider`, `…Declaration…`, `…TypeDefinition…`, `…Implementation…` | `DefinitionProvider`, `DeclarationProvider`, `TypeDefinitionProvider`, `ImplementationProvider` |
| `registerReferenceProvider` | `ReferenceProvider` |
| `registerDocumentHighlightProvider` | `DocumentHighlightProvider` |
| `registerRenameProvider` | `RenameProvider` |
| `registerCodeActionProvider` | `CodeActionProvider` |
| `registerCodeLensProvider` | `CodeLensProvider` |
| `registerInlayHintsProvider` | `InlayHintsProvider` |
| `registerInlineCompletionsProvider` | `InlineCompletionsProvider` |
| `registerDocumentFormattingEditProvider`, `…RangeFormatting…`, `…OnTypeFormatting…` | `DocumentFormattingProvider`, `DocumentRangeFormattingProvider`, `OnTypeFormattingProvider` |
| `registerDocumentSymbolProvider` | `DocumentSymbolProvider` |
| `registerFoldingRangeProvider` | `FoldingRangeProvider` |
| `registerSelectionRangeProvider` | `SelectionRangeProvider` |
| `registerLinkedEditingRangeProvider` | `LinkedEditingRangeProvider` |
| `registerLinkProvider` | `LinkProvider` |
| `registerColorProvider` | `DocumentColorProvider` |
| `registerDocumentSemanticTokensProvider`, `…RangeSemanticTokens…` | `DocumentSemanticTokensProvider`, `DocumentRangeSemanticTokensProvider` |
| `registerDocumentDropEditProvider`, `registerDocumentPasteEditProvider` | `DocumentDropEditProvider`, `DocumentPasteEditProvider` |
| `setLanguageConfiguration` | the language's configuration in the `LanguageRegistry` (`gpui-monaco-languages` reads Monaco's JSON) |
| `setMonarchTokensProvider` | a tree-sitter grammar with highlight queries (see [Languages](languages.md)) |

The `gpui-monaco-lsp` bridge registers these providers from a language server (see [Language servers](lsp.md)).

## Options

`IEditorOptions` becomes the typed `EditorOptions` struct with snake_case names, such as `wordWrap` → `word_wrap` and `minimap.enabled` → `minimap.enabled`. [Options](options.md) lists them with their Monaco names. The model-level options (`tabSize`, `insertSpaces`, …) are on `ModelOptions`.
