#!/bin/bash
# C55: the same failing command at the end of a script, once in a unit the
# bytecode VM runs and once in one too large for its operands — the latter
# silently goes to the tree-walking engine, and its errorInfo changes.
# usage: bash judge/audit-cases/big-unit.sh
cd "$(dirname "$0")/../.."
t=$(mktemp -d)
for n in 5000 12000; do
    {
        echo "set i 0"
        for ((k = 1; k < n; k++)); do echo 'for {set j 0} {$j < 1} {incr j} { set k $j }'; done
        echo 'for {set j 0} {$j < 1} {incr j} { set x [expr {1/$nosuchvar}] }'
    } > "$t/unit_$n.tcl"
    echo "=== $n loops (error on line $((n + 1)))"
    for who in "tclsh 8.6:tclsh" "tclsh 9.1:.refer/tcl910/inst/bin/tclsh9.1" "rtcl:target/release/rtcl -f"; do
        echo "  ${who%%:*}:"
        ${who#*:} "$t/unit_$n.tcl" 2>&1 | sed -n 5,6p | cut -c1-70 | sed 's/^/      /'
    done
done
rm -rf "$t"
