#!/bin/bash
# usage: bash judge/gates/bench/run.sh <bench.tcl> — µs/iter under tclsh, rtcl (bytecode), rtcl --no-bytecode; then peak RSS in KB
# worktree 相对；oracle tclsh 可用环境变量 BENCH_TCLSH 覆盖（默认 tclsh，即系统 8.6）。
cd "$(dirname "$0")/../.."
mkdir -p target/audit/bench
B=${BENCH_TCLSH:-tclsh}
f=$1
O=$(command -v -- "$B")
R=target/release/rtcl
/usr/bin/time -f "RSS %M" timeout 120 "$O" $f 2> target/audit/bench/.t.err > target/audit/bench/.t.out
/usr/bin/time -f "RSS %M" timeout 120 $R -f $f 2> target/audit/bench/.r.err > target/audit/bench/.r.out
/usr/bin/time -f "RSS %M" timeout 120 $R --no-bytecode -f $f 2> target/audit/bench/.n.err > target/audit/bench/.n.out
printf "%-18s %12s %12s %12s %8s\n" name "$B" rtcl-vm rtcl-tree "vm/$B"
paste -d'|' target/audit/bench/.t.out target/audit/bench/.r.out target/audit/bench/.n.out | grep -v DONE | awk -F'|' '{
  t=$2+0; r=$4+0; x=$6+0; ratio=(t>0)?r/t:0;
  printf "%-18s %12.2f %12.2f %12.2f %8.1f\n", $1, t, r, x, ratio }'
echo "peak RSS KB: $(grep -h RSS target/audit/bench/.t.err target/audit/bench/.r.err target/audit/bench/.n.err | tr '\n' ' ')"
