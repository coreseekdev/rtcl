# agent-e-loops: decoded tclsh 8.6.17 semantics for `for`'s next script.
# Oracle output (label|catchCode|result):
#   break in the next script  -> ends the loop, for completes with TCL_OK
#   continue in the next script -> ESCAPES the for (catch sees code 4):
#     the compiled next script has no in-loop continue target, so the
#     enclosing loop/handler sees it (for-8.2 .. for-8.12).
#   continue escaping to a proc body -> error 'invoked "continue" outside of a loop'
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }

t brk-next {set log {}; for {set i 0} {$i < 5} {incr i; break} { lappend log $i }; list $log $i}
t brk-next-eval {set log {}; for {set i 0} {$i < 5} {incr i; eval break} { lappend log $i }; list $log $i}
t cont-next {set log {}; for {set i 0} {$i < 5} {incr i; continue} { lappend log $i }}
t cont-next-rest {set log {}; for {set i 0} {$i < 5} {incr i; continue; set z 1} { lappend log $i }; list $log $i}
t cont-next-eval {set log {}; for {set i 0} {$i < 5} {incr i; eval continue} { lappend log $i }}
t cont-nested {apply {{} {
	for {set k 0} {$k < 3} {incr k} {
	    set j 0
	    for {set i 0} {$i < 5} {incr i; continue} { incr j; lappend log "k=$k i=$i j=$j" }
	    incr i
	}
	list $i $j $k
}}}
t cont-out-loop {continue}
t cont-in-proc {proc p {} { for {set i 0} {$i<3} {incr i; continue} {}; return after-$i }; p}
t cont-in-proc2 {proc p {} { for {set i 0} {$i<3} {incr i; continue} {} }; catch p}
t brk-in-proc {proc p {} { for {set i 0} {$i<3} {incr i; break} {}; return after-$i }; p}
t cont-next-ret0 {set l {}; for {set i 0} {$i<3} {incr i; return -level 0 x} {lappend l $i}; list done $l}
