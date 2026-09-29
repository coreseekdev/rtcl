if {[catch {::tcl::mathfunc::abs -0} m]} {puts "cmd ERR: $m"} else {puts "cmd: '$m'"}
if {[catch {tcl::mathfunc::abs -3} m]} {puts "cmd2 ERR: $m"} else {puts "cmd2: '$m'"}
