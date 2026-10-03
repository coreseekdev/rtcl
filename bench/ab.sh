#!/bin/bash
# bench/ab.sh — three-way A/B: tclsh vs a baseline rtcl binary vs the
# current tree's release binary, best-of-N over bench/cases/*.tcl.
#
#   bash bench/ab.sh [reps]              # default 5
#   BASE=/path/to/old-rtcl bash bench/ab.sh
#
# BASE defaults to the frozen pre-C5 worktree binary (the JIT milestone
# gate reference, bench/BASELINE.md table 2).  Recreate it after a /tmp
# wipe with:
#   git worktree add /tmp/rtcl-base-wt <pre-C5 sha> \
#     && git -C /tmp/rtcl-base-wt checkout <sha>   # if detached
#   cd /tmp/rtcl-base-wt && cargo build --release -p rtcl-cli
BASE=${BASE:-/tmp/rtcl-base-wt/target/release/rtcl}
NEW=${NEW:-$(cd "$(dirname "$0")/.." && pwd)/target/release/rtcl}
CASES="arith_loop dict_ops list_ops proc_fib string_build var_incr"
REPS=${1:-5}
cd "$(dirname "$0")/.." || exit 2
[ -x "$NEW" ] || { echo "ab: $NEW not built" >&2; exit 2; }
for c in $CASES; do
  f=bench/cases/$c.tcl
  bb=999999; nb=999999; tb=999999
  for i in $(seq $REPS); do
    if [ -x "$BASE" ]; then
      t0=$(date +%s%N); $BASE -f $f >/dev/null; t1=$(date +%s%N); [ $(( (t1-t0)/1000000 )) -lt $bb ] && bb=$(( (t1-t0)/1000000 ))
    fi
    t0=$(date +%s%N); $NEW -f $f >/dev/null; t1=$(date +%s%N); [ $(( (t1-t0)/1000000 )) -lt $nb ] && nb=$(( (t1-t0)/1000000 ))
    t0=$(date +%s%N); tclsh $f >/dev/null; t1=$(date +%s%N); [ $(( (t1-t0)/1000000 )) -lt $tb ] && tb=$(( (t1-t0)/1000000 ))
  done
  if [ -x "$BASE" ]; then
    printf "%-14s tclsh %4d  base %4d  new %4d  vs-base %s  vs-tclsh %s\n" "$c" "$tb" "$bb" "$nb" "$(python3 -c "print(f'{$bb/$nb:.2f}x')")" "$(python3 -c "print(f'{$nb/$tb:.2f}x')")"
  else
    printf "%-14s tclsh %4d  new %4d  vs-tclsh %s\n" "$c" "$tb" "$nb" "$(python3 -c "print(f'{$nb/$tb:.2f}x')")"
  fi
done
