#!/bin/bash
# 提交纪律的余下部分：sweep 与 workspace 测试。用法：bash judge/gates/rest-gates.sh <tag>
# 在仓库任何 worktree 里都可用；日志写在 <worktree>/target/audit/（git-ignored）。
cd "$(dirname "$0")/../.."
mkdir -p target/audit
log=target/audit/rest-$1.log
: > $log
run() { local name=$1; shift; local t0=$(date +%s); echo "=== $name" >> $log; "$@" >> $log 2>&1; local rc=$?; echo "=== $name rc=$rc ($(( $(date +%s) - t0 ))s)" >> $log; }
run sweep bash judge/sweep.sh
run workspace bash -c 'ulimit -v 6000000; cargo test --workspace --no-fail-fast -j 2 2>&1 | grep -E "^test result|FAILED|panicked|^error|Running" | grep -v fn_addr_eq'
echo ALLDONE >> $log

