set __judge_c [catch {
    set x {}
    for {set i 0} {$i<100000} {incr i} {
	set x [list $x {}]
    }
    unset x
} __judge_r]
puts MARKER
