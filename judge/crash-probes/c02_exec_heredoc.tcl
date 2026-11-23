# crash-probe: c02 exec heredoc
# gate: max-ms 1500
source [file join [file dirname [info script]] _chk.tcl]
# gate: skip-tree
set t0 [clock milliseconds]
set r [catch {exec tr x y << [string repeat x 200000]} e]
chk rc $r 0
chk len [string length $e] 200000
