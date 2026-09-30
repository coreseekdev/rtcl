namespace eval test_ns_var {
	set : 123
	set v: 456
	set x:y: 789
	list [set :] [set v:] [set x:y:] ${:} ${v:} ${x:y:} [expr {":" in [info vars]}] [expr {"v:" in [info vars]}] [expr {"x:y:" in [info vars]}]
}
puts "R<<[set res [namespace eval test_ns_var {
	set : 123
	set v: 456
	set x:y: 789
	list [set :] [set v:] [set x:y:] ${:} ${v:} ${x:y:} [expr {":" in [info vars]}] [expr {"v:" in [info vars]}] [expr {"x:y:" in [info vars]}]
}]]>>"
