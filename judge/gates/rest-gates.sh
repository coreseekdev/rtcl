#!/bin/bash
# 提交纪律的余下部分：sweep 与 workspace 测试。用法：bash judge/gates/rest-gates.sh <tag>
# 在仓库任何 worktree 里都可用；日志写在 <worktree>/target/audit/（git-ignored）。
cd "$(dirname "$0")/../.."
mkdir -p target/audit
log=target/audit/rest-$1.log
: > $log
run() { local name=$1; shift; local t0=$(date +%s); echo "=== $name" >> $log; "$@" >> $log 2>&1; local rc=$?; echo "=== $name rc=$rc ($(( $(date +%s) - t0 ))s)" >> $log; }
run sweep bash judge/sweep.sh
# workspace 测试**不带 ulimit**：tk-demo 的大链接在 6M 上限下必失败
# （2026-11-26 两次实证）；sweep 的 ulimit 坑同款经验——测试阶段去上限。
# rc 取 cargo 本尊（管道进 grep 会把 rc 换成 grep 的，链接失败假绿过门
# 一次），摘要 grep 只做展示。
run workspace bash -c 'out=$(cargo test --workspace --no-fail-fast -j 2 2>&1); rc=$?; printf "%s\n" "$out" | grep -E "^test result|FAILED|panicked|^error" | grep -v fn_addr_eq | tail -80; exit $rc'
echo ALLDONE >> $log

