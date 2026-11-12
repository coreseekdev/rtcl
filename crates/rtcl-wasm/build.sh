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

# Binaryen -Oz：LLVM 之后的第二轮尺寸优化（-8.5% raw；函数体内存访问
# 模式/不可达分支清理）。--all-features 因 trunc_sat/simd 等 MVP 后指
# 令已在产物中出现。跳过：WASM_NO_OPT=1 ./build.sh
if command -v wasm-opt >/dev/null && [[ "${WASM_NO_OPT:-}" != "1" ]]; then
    wasm-opt -Oz --all-features pkg/rtcl_wasm_bg.wasm \
        -o pkg/rtcl_wasm_bg.wasm.tmp \
        && mv pkg/rtcl_wasm_bg.wasm.tmp pkg/rtcl_wasm_bg.wasm
    echo "wasm-opt -Oz applied: $(stat -c%s pkg/rtcl_wasm_bg.wasm) bytes"
fi

echo "pkg/ generated: $(ls pkg | tr '\n' ' ')"
