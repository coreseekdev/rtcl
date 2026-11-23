# crash-probe: c01 exec stderr deadlock
# gate: max-ms 1500
source [file join [file dirname [info script]] _chk.tcl]
# gate: skip-tree
set t0 [clock milliseconds]
set r [catch {exec sh -c {echo OUT; head -c 66000 /dev/zero | tr '\0' E >&2}} e]
chk rc $r 0
chk ms [expr {[clock milliseconds] - $t0 < 5000}] 1
