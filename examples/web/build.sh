#!/usr/bin/env bash
# Builds the wasm module into www/pkg. Needs the toolchain gpui's web platform
# builds with and the matching wasm-bindgen CLI (see README.md).
#
#   ./build.sh [--release] [--font path/to/Mono.ttf]
#
# --font copies a monospace TrueType/OpenType font to www/fonts/editor.ttf,
# which the page loads (gpui's web platform ships no fonts).
set -euo pipefail
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$dir/../.." && pwd)"
profile=debug
args=(+nightly build -p web --target wasm32-unknown-unknown)
font=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --release) profile=release; args+=(--release) ;;
    --font) font="${2:?--font needs a path}"; shift ;;
    *) echo "usage: $0 [--release] [--font path/to/font.ttf]" >&2; exit 2 ;;
  esac
  shift
done

# The CLI must match the wasm-bindgen crate in Cargo.lock exactly.
want="$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/version = |"/, ""); print; exit }' "$root/Cargo.lock")"
if ! have="$(wasm-bindgen --version 2>/dev/null)"; then
  echo "wasm-bindgen CLI not found: cargo install wasm-bindgen-cli --locked --version $want" >&2
  exit 1
fi
if [[ "$have" != "wasm-bindgen $want" ]]; then
  echo "$have doesn't match the wasm-bindgen $want in Cargo.lock:" >&2
  echo "  cargo install wasm-bindgen-cli --locked --version $want" >&2
  exit 1
fi

cargo "${args[@]}"
wasm-bindgen "$root/target/wasm32-unknown-unknown/$profile/web.wasm" \
  --out-dir "$dir/www/pkg" --target web --no-typescript

if [[ -n "$font" ]]; then
  mkdir -p "$dir/www/fonts"
  cp "$font" "$dir/www/fonts/editor.ttf"
fi
if [[ ! -f "$dir/www/fonts/editor.ttf" ]]; then
  echo "note: no font at www/fonts/editor.ttf; pass --font path/to/Mono.ttf (see README.md)" >&2
fi
echo "built www/; serve it with: python3 -m http.server -d $dir/www"
