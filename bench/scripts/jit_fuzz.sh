#!/usr/bin/env bash
# JIT 差分模糊测试 runner：生成 → 接受性探针（REJECT 丢弃并计数）→
# tclsh / rtcl / jitsh 三方逐字节比（stdout+stderr+rc）。
# 用法: jit_fuzz.sh --n 500 --seed 0 [--engine wasmi|wasmtime]
# 分歧脚本留在 bench/scripts/jit_fuzz_findings/（人工最小化前的原始样本）。
set -u
cd "$(dirname "$0")/../.."
N=500; SEED=0; ENGINE=${RTCL_JIT_ENGINE:-wasmi}
while [ $# -gt 0 ]; do
  case "$1" in
    --n) N=$2; shift 2;;
    --seed) SEED=$2; shift 2;;
    --engine) ENGINE=$2; shift 2;;
    *) shift;;
  esac
done
RTCL=./target/release/rtcl
JIT=./target/release/jitsh
TMP=$(mktemp -d /tmp/jitfuzz.XXXXXX)
trap 'rm -rf "$TMP"' EXIT
python3 bench/scripts/jit_fuzz.py --n "$N" --seed "$SEED" --out "$TMP" >/dev/null

mkdir -p bench/scripts/jit_fuzz_findings
gen=0; acc=0; rej=0; run=0; div=0
for f in "$TMP"/*.tcl; do
    gen=$((gen+1))
    v=$(RTCL_JIT_ACCEPT=1 "$JIT" -c "$(cat "$f")" 2>/dev/null)
    case "$v" in
        ACCEPT) acc=$((acc+1));;
        *) rej=$((rej+1)); continue;;
    esac
    run=$((run+1))
    o=$(timeout 10 tclsh "$f" 2>&1); oc=$?
    r=$(timeout 10 "$RTCL" -f "$f" 2>&1); rc2=$?
    if [ "$ENGINE" = wasmtime ]; then
        j=$(RTCL_JIT_ENGINE=wasmtime timeout 10 "$JIT" -f "$f" 2>&1); jc=$?
    else
        j=$(timeout 10 "$JIT" -f "$f" 2>&1); jc=$?
    fi
    if [ "$r" != "$j" ] || [ "$rc2" != "$jc" ]; then
        div=$((div+1))
        # 只收 jit≠rtcl 且 rtcl==tclsh 的真分歧；预存（rtcl≠tclsh）单独计数不收
        if [ "$o" = "$r" ] && [ "$oc" = "$rc2" ]; then
            cp "$f" "bench/scripts/jit_fuzz_findings/real_${ENGINE}_$(basename "$f")"
        else
            cp "$f" "bench/scripts/jit_fuzz_findings/preexist_${ENGINE}_$(basename "$f")"
        fi
    fi
done
echo "fuzz($ENGINE): generated=$gen accepted=$acc rejected=$rej ran=$run divergences=$div"
