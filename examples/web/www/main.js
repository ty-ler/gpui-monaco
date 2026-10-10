// Loads the wasm module built by ../build.sh and starts the editor with a
// monospace font you place at fonts/editor.ttf (the web platform has none).
const FONT_URL = './fonts/editor.ttf';
// The family name inside that font file.
const FONT_FAMILY = new URLSearchParams(location.search).get('family') ?? 'JetBrains Mono';

const status = document.getElementById('status');
try {
  const wasm = await import('./pkg/web.js');
  await wasm.default();
  const response = await fetch(FONT_URL);
  if (!response.ok) throw new Error(`no font at ${FONT_URL}`);
  const font = new Uint8Array(await response.arrayBuffer());
  wasm.run(font, FONT_FAMILY);
  status.remove();
} catch (error) {
  console.error(error);
  status.textContent = `Failed to start: ${error.message ?? error}`;
}
