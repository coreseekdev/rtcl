#!/usr/bin/env bash
# Build rtcl-wasm for the web: cargo -> wasm32 + wasm-bindgen JS bindings.
# Usage: ./build.sh [--dev]
set -euo pipefail
cd "$(dirname "$0")"

PROFILE=release
FLAGS=(--release)
if [[ "${1:-}" == "--dev" ]]; then
    PROFILE=debug
    FLAGS=()
fi

cargo build -p rtcl-wasm --target wasm32-unknown-unknown "${FLAGS[@]}"

rm -rf pkg
wasm-bindgen --target web \
    --out-dir pkg --out-name rtcl_wasm \
    "../../target/wasm32-unknown-unknown/${PROFILE}/rtcl_wasm.wasm"

echo "pkg/ generated: $(ls pkg | tr '\n' ' ')"
