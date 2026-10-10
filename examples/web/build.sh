#!/usr/bin/env bash
# Builds the wasm module into www/pkg. Needs the toolchain gpui's web platform
# builds with and the matching wasm-bindgen CLI (see README.md).
set -euo pipefail
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$dir/../.." && pwd)"
profile=debug
args=(+nightly build -p web --target wasm32-unknown-unknown)
if [[ "${1:-}" == "--release" ]]; then profile=release; args+=(--release); fi
cargo "${args[@]}"
wasm-bindgen "$root/target/wasm32-unknown-unknown/$profile/web.wasm" \
  --out-dir "$dir/www/pkg" --target web --no-typescript
