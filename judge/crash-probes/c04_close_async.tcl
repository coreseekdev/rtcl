# crash-probe: c04 close async
# gate: max-ms 1500
source [file join [file dirname [info script]] _chk.tcl]
# gate: skip-tree
set s [socket -async 10.255.255.1 80]
set t0 [clock milliseconds]
close $s
chk ms [expr {[clock milliseconds] - $t0 < 3000}] 1
