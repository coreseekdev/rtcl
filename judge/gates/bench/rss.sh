#!/bin/bash
# peak RSS (KB) of a script under tclsh and rtcl（worktree 相对）
cd "$(dirname "$0")/../.."
/usr/bin/time -f "tclsh RSS %M KB  %e s" tclsh "$1" 2>&1 | grep -v DONE
/usr/bin/time -f "rtcl  RSS %M KB  %e s" target/release/rtcl -f "$1" 2>&1 | grep -v DONE
