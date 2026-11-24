# crash-probe: c06 expr hash
# gate: max-ms 3000
# `#` in an expression is an invalid character in tclsh 8.6.  The checker
# scanned it as a comment and restarted without advancing, so every
# `expr {3 #}` / `expr {#}` / `expr {1 + #}` spun at 100% CPU forever.
source [file join [file dirname [info script]] _chk.tcl]
set ::errorCode NONE
set r [catch {expr {3 #}} m]
chk rc $r 1
chk msg $m {invalid character "#"
in expression "3 #"}
chk ec $::errorCode {TCL PARSE EXPR BADCHAR}
chk alone [catch {expr {#}} m] 1
chk operand [catch {expr {1 + #}} m] 1
chk multiline [catch {expr {# c
1 + 1}} m] 1
# inside a string, a brace or a script substitution it is data
chk quoted [expr {"a # b"}] {a # b}
chk script [expr {3 + [llength {# x}]}] 5
chk braced [expr {[string length {# #}]}] 3
