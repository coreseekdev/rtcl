# crash-probe: c01 exec stderr deadlock
# gate: max-ms 1500
source [file join [file dirname [info script]] _chk.tcl]
# gate: skip-tree
# A child's stderr was read only after its stdout reached EOF; a pipe's
# worth on stderr while stdout was still open blocked both for good.
set t0 [clock milliseconds]
set r [catch {exec sh -c {echo OUT; head -c 66000 /dev/zero | tr '\0' E >&2}} e]
# tclsh: what the child wrote to stderr fails the exec and is the error's
# message, after its stdout.
chk rc $r 1
chk msg-len [string length $e] 66004
chk head [string range $e 0 4] {OUT
E}
chk tail [string range $e end-1 end] EE
chk ms [expr {[clock milliseconds] - $t0 < 1500}] 1
