# crash-probe: b19 regexp capture word
source [file join [file dirname [info script]] _chk.tcl]
set t0 [clock milliseconds]
set r [catch {regexp {(\w+)$} [string repeat a 15000]} e]
chk rc $r 0
chk match $e 1
chk ms [expr {[clock milliseconds] - $t0 < 5000}] 1
