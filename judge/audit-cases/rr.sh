#!/bin/bash
# usage: rr.sh <script.tcl> [rtcl-binary] — the same script under four
# interpreters, each on one line: tclsh 8.6.17, tclsh 9.1.0 (the oracle,
# built in the repository), rtcl (bytecode), rtcl --no-bytecode (tree).
# Diff the lines; what the two tclsh disagree on is 8.6-vs-9.1 semantics,
# not an rtcl defect.
cd "$(dirname "$0")/../.."
f=$1; bin=${2:-target/release/rtcl}
T91=.refer/tcl910/inst/bin/tclsh9.1
echo "86:    $(tclsh $f 2>&1 | tr '\n' '|')"
[ -x $T91 ] && echo "91:    $($T91 $f 2>&1 | tr '\n' '|')"
echo "vm:    $($bin -f $f 2>&1 | tr '\n' '|')"
echo "tree:  $($bin --no-bytecode -f $f 2>&1 | tr '\n' '|')"
