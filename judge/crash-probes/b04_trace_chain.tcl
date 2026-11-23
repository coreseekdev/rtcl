# crash-probe: b04 trace chain
source [file join [file dirname [info script]] _chk.tcl]
for {set i 0} {$i < 30000} {incr i} {
    trace add variable a$i write "set a[expr {$i+1}] 1;#"
}
set r [catch {set a0 1} e]
chk rc $r 1
