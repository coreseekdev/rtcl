#!/bin/bash
# 门禁速跑集（quick gates），顺序跑进一个日志。用法：bash judge/gates/quick-gates.sh <tag>
# 在仓库任何 worktree 里都可用；日志写在 <worktree>/target/audit/（git-ignored）。
cd "$(dirname "$0")/../.."
mkdir -p target/audit
tag=$1
log=target/audit/quick-$tag.log
: > $log
run() { local name=$1; shift; local t0=$(date +%s); echo "=== $name" >> $log; "$@" >> $log 2>&1; local rc=$?; echo "=== $name rc=$rc ($(( $(date +%s) - t0 ))s)" >> $log; }
run judge bash judge/run.sh
run crash-gate bash judge/crash-gate.sh
run probes bash judge/probes.sh
run vm-lib bash -c 'ulimit -v 3400000; cargo test -p rtcl-vm --lib 2>&1 | grep -E "^test result|FAILED|panicked" | grep -v fn_addr_eq'
run vm-lib-nostd bash -c 'ulimit -v 3400000; cargo test -p rtcl-vm --no-default-features --features rtcl_std --lib 2>&1 | grep -E "^test result|FAILED|panicked" | grep -v fn_addr_eq'
run wasm-lib bash -c 'ulimit -v 3400000; cargo test -p rtcl-wasm 2>&1 | grep -E "^test result|FAILED|panicked|^error" | grep -v fn_addr_eq'
# A compile error must fail the step: the rc of a pipeline into grep/head
# is grep's, not cargo's (a broken wasm build passed here once).
count_errors() { grep -cE "^error(\[|:)" || true; }
run check-iofile bash -c 'ulimit -v 3400000; out=$(cargo check -p rtcl-vm --no-default-features --features io,file 2>&1); echo "$out" | grep -E "^error" -A6 | head -30; echo "$out" | grep -E "Finished"; [ "$(echo "$out" | grep -cE "^error(\[|:)")" = 0 ]'
run check-noregexp bash -c 'ulimit -v 3400000; out=$(cargo check -p rtcl-vm --no-default-features --features io,file,package,env,clock,encoding,json 2>&1); echo "$out" | grep -E "^error" -A6 | head -30; echo "$out" | grep -E "Finished"; [ "$(echo "$out" | grep -cE "^error(\[|:)")" = 0 ]'
run check-lite bash -c 'ulimit -v 3400000; out=$(cargo check -p rtcl-vm --no-default-features --features regexp-lite,io,file 2>&1); echo "$out" | grep -E "^error" -A6 | head -30; echo "$out" | grep -E "Finished"; [ "$(echo "$out" | grep -cE "^error(\[|:)")" = 0 ]'
run check-nospencer bash -c 'ulimit -v 3400000; out=$(cargo check -p rtcl-vm --no-default-features --features regexp,io,file 2>&1); echo "$out" | grep -E "^error" -A6 | head -30; echo "$out" | grep -E "Finished"; [ "$(echo "$out" | grep -cE "^error(\[|:)")" = 0 ]'
run wasm32 bash -c 'ulimit -v 3400000; out=$(cargo build -p rtcl-wasm --target wasm32-unknown-unknown -j 2 2>&1); echo "$out" | grep -E "^error" -A6 | head -30; echo "$out" | grep -E "Finished"; [ "$(echo "$out" | grep -cE "^error(\[|:)")" = 0 ]'
echo ALLDONE >> $log

