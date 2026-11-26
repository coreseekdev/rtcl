# lfilter (TIP 735, tclsh 9.1) — behavior pin against the 9.1 oracle.
# Source of truth: doc/lfilter.n + tclCmdAH.c EachloopCmd(TCL_EACH_FILTER).
puts [lfilter x {1 2 3 4 5 6} {expr {$x > 2 && $x < 5}}]
puts [lfilter y {a b c d e f} x {1 2 3 4 5 6} {expr {$x > 2 && $x < 5}}]
puts [lfilter {x y} {1 2 3 4 5 6 7 8} {expr {$x > 2 && $x < 7}}]
puts [lfilter x {1 2 3 4 5} y {2 2 2} {expr {$x > $y}}]
# wrong-args face (tclCmdAH.c: objc<4 or odd -> WrongNumArgs)
catch {lfilter} m; puts $m
catch {lfilter a b} m; puts $m
catch {lfilter a b c d} m; puts $m
# body error propagates
catch {lfilter x 0 {error gorp}} m; puts $m
# break stops without collecting; continue skips collection
puts [lfilter x {1 2 3 4 5} {if {$x == 2} break {expr 1}}]
puts [lfilter x {1 2 3 4 5} {if {$x == 2} continue {expr 1}}]
# body result must be a boolean (Tcl_GetBooleanFromObj)
catch {lfilter x {1 2} {string cat gorp}} m; puts $m
puts [lfilter x {} {string cat gorp}]
# empty varlist -> NEEDVARS
catch {lfilter {} {1 2} {expr 1}} m; puts "$m :: $errorCode"
# collected value is the FIRST variable of the FIRST varlist
puts [lfilter {p q} {a b c d} {string equal $p $q}]
# boolean accept set: yes/no/true/false/0/1
puts [lfilter x {a b c d e f} {string match {[bc]} $x}]
# inside apply (compiled path): collect-on-true, break, continue
puts [apply {{} {
    set r [lfilter v {1 3 5 6 7} {expr {$v % 2}}]
    return $r
}}]
puts [apply {{} {
    lfilter v {1 2 3 4} {if {$v == 2} break; expr 1}
}}]
# dispatched from inside a proc with dynamic body var
set body {expr {$x >= 2}}
puts [apply {{body} {
    lfilter x {1 2 3} $body
}} $body]
# loop variable persists after the loop (foreach semantics)
lfilter k {a b c} {expr 1}
puts $k
# list-element errors surface through strict list parsing
catch {lfilter x {{1 2}3} {expr 1}} m; puts "$m :: $errorCode"
catch {lfilter {{x y}} {1 2 3} {expr 1}} m; puts $m
