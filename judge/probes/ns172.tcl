puts [catch {
    namespace eval test_ns_1 {
	variable x 777
	set ::test_ns_1::x
    }
} r]
puts $r
