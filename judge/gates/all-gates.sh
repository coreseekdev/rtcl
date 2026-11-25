#!/bin/bash
# quick 集 + sweep + workspace 测试，分两个日志。用法：bash judge/gates/all-gates.sh <tag>
# 在仓库任何 worktree 里都可用；日志写在 <worktree>/target/audit/（git-ignored）。
cd "$(dirname "$0")/../.."
mkdir -p target/audit
tag=$1
bash judge/gates/quick-gates.sh $tag
log=target/audit/rest-$tag.log
: > $log
run() { local name=$1; shift; local t0=$(date +%s); echo "=== $name" >> $log; "$@" >> $log 2>&1; echo "=== $name rc=$? ($(( $(date +%s) - t0 ))s)" >> $log; }
run sweep bash judge/sweep.sh
run workspace-build bash -c 'ulimit -v 6000000; cargo test --workspace --no-fail-fast -j 2 --no-run'
run workspace cargo test --workspace --no-fail-fast -j 2
echo ALLDONE >> $log

