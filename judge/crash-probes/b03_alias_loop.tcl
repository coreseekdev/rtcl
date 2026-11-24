# crash-probe: b03 alias loop
source [file join [file dirname [info script]] _chk.tcl]
# tclsh rejects the definition: cannot define or rename alias "a": would create a loop
chk self-def [catch {alias a a} e] 1
chk self-msg $e {cannot define or rename alias "a": would create a loop}
chk self-call [catch {a} e] 1
alias b puts
chk chain-def [catch {alias c b}] 0
chk mutual-def [catch {alias b c} e] 1
chk mutual-msg $e {cannot define or rename alias "b": would create a loop}
# a real chain still works (each alias call nests one level and unwinds)
proc leaf {x} { return "got $x" }
alias c0 leaf
for {set i 1} {$i < 300} {incr i} { "alias" "c$i" "c[expr {$i - 1}]" }
chk chain [c299 end] {got end}
