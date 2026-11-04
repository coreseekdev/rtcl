#!/usr/bin/env bash
# tclmem.sh: 在测试护栏 cgroup 下运行命令（用户规约，2026-11-05）。
#
#   MemoryMax=512M, MemorySwapMax=0, OOMScoreAdjust=1000,
#   RuntimeMaxSec=${TCLMEM_SECS:-180}
#
# 一切 tclsh/rtcl 测试调用（probe 套件、judge、单测）都应经此包装——
# D-36 崩溃轮证明不受限的解释器可能把整机拖垮（栈溢出 abort 只是
# 其一）。用例：
#   specs/tclmem.sh bash specs/probe.sh < specs/trace-deep.probes
#   TCLMEM_SECS=300 specs/tclmem.sh bash judge/run.sh
set -euo pipefail
exec systemd-run --user --wait --pipe \
    -p MemoryMax=512M -p MemorySwapMax=0 \
    -p OOMScoreAdjust=1000 -p "RuntimeMaxSec=${TCLMEM_SECS:-180}" \
    --working-directory="$PWD" \
    bash -c "$*"
