# crash-probe: b13 expr braced compile
source [file join [file dirname [info script]] _chk.tcl]
set n 100000
set body "expr {" 
append body [string repeat "(" $n] 1 [string repeat ")" $n] "}"
set r [catch {proc p {} $body} e]
chk rc $r 1
