# crash-probe: d13 destructor cascade
source [file join [file dirname [info script]] _chk.tcl]
set n 0
oo::class create C { destructor {incr ::n; C destroy} }
set o [C new]
catch {$o destroy}
chk runs $n 1
