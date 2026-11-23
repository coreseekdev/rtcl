# crash-probe: d06 trace after bump
source [file join [file dirname [info script]] _chk.tcl]
set x 0
set y 0
set fired 0
rename incr {}
trace add variable x write {set fired [expr {$fired + 50}];#}
set x 1
set x 2
chk fired $fired 100
