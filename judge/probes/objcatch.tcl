set __judge_c [catch {
    set x {}
    for {set i 0} {$i<100000} {incr i} {
	set x [list $x {}]
    }
    unset x
} __judge_r]
if {$__judge_r eq {}} { puts "c=$__judge_c r=EMPTY" } else { puts "c=$__judge_c r=[string length $__judge_r]" }
