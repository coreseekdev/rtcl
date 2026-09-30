namespace eval test_ns_hier1 {
	set test_ns_level 1
	namespace eval test_ns_hier2 {
	    set test_ns_level 2
	}
}
puts "R<<[set test_ns_hier1::test_ns_level] [set test_ns_hier1::test_ns_hier2::test_ns_level]>>"
