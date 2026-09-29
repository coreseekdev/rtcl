#!/usr/bin/env bash
# probe.sh: run each snippet (one per line, \n escaped) through tclsh and rtcl, diff.
set -u
cd "$(dirname "$0")/.."
RTCL=./target/release/rtcl
while IFS= read -r line; do
    [ -z "$line" ] && continue
    case "$line" in \#*) continue;; esac
    snippet=$(printf '%b' "$line")
    o_out=$(tclsh <<< "$snippet" 2>&1); o_code=$?
    r_out=$("$RTCL" -c "$snippet" 2>&1); r_code=$?
    if [ "$o_out" = "$r_out" ] && [ "$o_code" = "$r_code" ]; then
        echo "SAME  | $line"
    else
        echo "DIFF  | $line"
        echo "  tclsh($o_code): $o_out"
        echo "  rtcl ($r_code): $r_out"
    fi
done
