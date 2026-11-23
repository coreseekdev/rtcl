# crash-probe: b03 alias loop
source [file join [file dirname [info script]] _chk.tcl]
catch {alias a a}
chk self [catch {a} e] 1
alias b c
alias c b
chk mutual [catch {b} e] 1
