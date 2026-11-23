# crash-probe: b18 regexp capture big
source [file join [file dirname [info script]] _chk.tcl]
set t0 [clock milliseconds]
set r [catch {regexp {(.*)x} [string repeat a 30000]x} e]
chk rc $r 0
chk match $e 1
chk ms [expr {[clock milliseconds] - $t0 < 5000}] 1
