set __judge_c [catch {
    set x {}
    for {set i 0} {$i<100000} {incr i} {
	set x [list $x {}]
    }
    unset x
} __judge_r]
if {$__judge_c == 0 && $__judge_r eq {}} {puts PASS} else {puts "FAIL obj-32.1"}
