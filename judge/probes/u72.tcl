namespace eval test_ns_uplevel {
    variable x 0
    variable y 1
    proc show_vars {num} {
	return [uplevel $num {info vars}]
    }
    proc test_uplevel {num} {
	set a 0
	set b 1
	namespace eval ::test_ns_uplevel " return \[show_vars $num\] "
    }
}
puts "R<<[test_ns_uplevel::test_uplevel 1]>>"
