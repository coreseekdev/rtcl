#!/usr/bin/env bash
# Build rtcl-wasm for the web: cargo -> wasm32 + wasm-bindgen JS bindings.
# Usage: ./build.sh [--dev] [--size] [--core]
set -euo pipefail
cd "$(dirname "$0")"

PROFILE=release
FLAGS=(--release)
SIZE_TRACK=0
CORE_TRACK=0
for arg in "$@"; do
    case "$arg" in
        --dev) PROFILE=debug; FLAGS=() ;;
        --size) SIZE_TRACK=1 ;;
        --core) CORE_TRACK=1 ;;
        *) echo "usage: $0 [--dev] [--size] [--core]" >&2; exit 2 ;;
    esac
done
if [[ $SIZE_TRACK == 1 && "$PROFILE" != release ]]; then
    echo "--size requires the release profile（debug 面无尺寸意义）" >&2
    exit 2
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

# ── size 轨（wasi 招②增量，docs/MODULARIZATION.md §5）──────────────
# 第二产物 profile wasm-size（inherits release，opt-level "s"）：尺寸
# 优先的构建变体——运行时代价 −40-50%（wasi 线 opt3→opt-s 同族实测
# 参照），只用于尺寸敏感的嵌入分发；dispatcher 真机面恒走 perf 轨。
# 只取 .wasm 作第二产物（*_size.wasm 后缀区分）：JS 胶水两轨同面
# （同 exports/imports），部署 = 同一份 rtcl_wasm.js 配 *_size.wasm
# 换名即用。两轨都过 wasm-opt -Oz。
if [[ $SIZE_TRACK == 1 ]]; then
    cargo build -p rtcl-wasm --target wasm32-unknown-unknown --profile wasm-size
    SIZE_TMP=$(mktemp -d)
    trap 'rm -rf "$SIZE_TMP"' EXIT
    wasm-bindgen --target web \
        --out-dir "$SIZE_TMP" --out-name rtcl_wasm \
        "../../target/wasm32-unknown-unknown/wasm-size/rtcl_wasm.wasm"
    cp "$SIZE_TMP/rtcl_wasm_bg.wasm" pkg/rtcl_wasm_bg_size.wasm
    if command -v wasm-opt >/dev/null && [[ "${WASM_NO_OPT:-}" != "1" ]]; then
        wasm-opt -Oz --all-features pkg/rtcl_wasm_bg_size.wasm \
            -o pkg/rtcl_wasm_bg_size.wasm.tmp \
            && mv pkg/rtcl_wasm_bg_size.wasm.tmp pkg/rtcl_wasm_bg_size.wasm
    fi
    echo "── size 轨数字链（pkg/rtcl_wasm_bg_size.wasm；运行时 −40-50%）──"
    echo "raw: $(stat -c%s '../../target/wasm32-unknown-unknown/wasm-size/rtcl_wasm.wasm') bytes"
    echo "opt: $(stat -c%s pkg/rtcl_wasm_bg_size.wasm) bytes"
    echo "gzip: $(gzip -c9 pkg/rtcl_wasm_bg_size.wasm | wc -c) bytes"
fi

echo "── perf 轨数字链（pkg/rtcl_wasm_bg.wasm）──"
echo "raw: $(stat -c%s "../../target/wasm32-unknown-unknown/${PROFILE}/rtcl_wasm.wasm") bytes"
if [[ -f pkg/rtcl_wasm_bg.wasm ]]; then
    echo "opt: $(stat -c%s pkg/rtcl_wasm_bg.wasm) bytes"
    echo "gzip: $(gzip -c9 pkg/rtcl_wasm_bg.wasm | wc -c) bytes"
fi

# ── core 轨（D6 Core Profile，specs/ARCH-DESIGN-20261126.md）──────────
# features core = std,io,dynmod（no file/json/ext + 无 ui/http/log/rules
# 词表层）：网页应用最小化 command script 核心——核心语言 + expr +
# list/dict/string + channel 内存面 + OO。体积红线：opt ≤1.5MB。
# profile 默认 wasm-size（opt-s，MODULARIZATION §5 同款口径：尺寸敏感
# 的嵌入分发承担 −40-50% 运行时代价）；CORE_PROFILE=release 可回 perf 档。
# 产物 pkg/rtcl_wasm_bg_core.wasm；JS 胶水两轨同面（同 exports/
# imports），部署 = 同一份 rtcl_wasm.js 换 .wasm 名即用。std 默认轨
# 照常先行——pkg/ 恒有全量面供标准门禁。
if [[ $CORE_TRACK == 1 ]]; then
    CORE_PROFILE="${CORE_PROFILE:-wasm-size}"
    cargo build -p rtcl-wasm --target wasm32-unknown-unknown \
        --profile "$CORE_PROFILE" --no-default-features --features core
    CORE_TMP=$(mktemp -d)
    trap 'rm -rf "$CORE_TMP"' EXIT
    wasm-bindgen --target web \
        --out-dir "$CORE_TMP" --out-name rtcl_wasm \
        "../../target/wasm32-unknown-unknown/${CORE_PROFILE}/rtcl_wasm.wasm"
    cp "$CORE_TMP/rtcl_wasm_bg.wasm" pkg/rtcl_wasm_bg_core.wasm
    if command -v wasm-opt >/dev/null && [[ "${WASM_NO_OPT:-}" != "1" ]]; then
        wasm-opt -Oz --all-features pkg/rtcl_wasm_bg_core.wasm \
            -o pkg/rtcl_wasm_bg_core.wasm.tmp \
            && mv pkg/rtcl_wasm_bg_core.wasm.tmp pkg/rtcl_wasm_bg_core.wasm
    fi
    echo "── core 轨数字链（pkg/rtcl_wasm_bg_core.wasm；红线 opt ≤1.5MB）──"
    echo "raw: $(stat -c%s "../../target/wasm32-unknown-unknown/${CORE_PROFILE}/rtcl_wasm.wasm") bytes"
    echo "opt: $(stat -c%s pkg/rtcl_wasm_bg_core.wasm) bytes"
    echo "gzip: $(gzip -c9 pkg/rtcl_wasm_bg_core.wasm | wc -c) bytes"
fi
echo "pkg/ generated: $(ls pkg | tr '\n' ' ')"
