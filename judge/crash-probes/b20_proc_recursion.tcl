# crash-probe: b20 proc recursion
source [file join [file dirname [info script]] _chk.tcl]
proc f {n} { if {$n <= 0} {return 0}; expr {[f [expr {$n - 1}]] + 1} }
chk rc [catch {f 999} e] 1
chk rc2 [catch {f 100000} e] 1
