namespace eval test_ns_simple {
	variable test_ns_x 0
	proc test {test_ns_x} {
	    return "test: $test_ns_x"
	}
}
puts "BODY<<[info body test_ns_simple::test]>>"
puts "TRIM<<[string trim [info body test_ns_simple::test]]>>"
