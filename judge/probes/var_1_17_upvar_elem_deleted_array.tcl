set __judge_c [catch {
    namespace eval test_ns_var {
	variable result
	proc p {} {
	    array set x {1 2 3 4}
	    upvar 0 x(1) foo
	    lappend result [catch {set foo} msg] $msg
	    unset x
	    lappend result [catch {set foo 3} msg] $msg
	}
	set result [p]
	namespace delete [namespace current]
	set result
    }
} __judge_r]
puts "c=$__judge_c r= $__judge_r"
