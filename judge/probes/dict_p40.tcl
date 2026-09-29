puts "ns: [namespace eval tcl::mathop {info commands}]"
puts "plus: [info commands ::tcl::mathop::*]"
catch {::tcl::mathop::+ 1 2} r; puts "abs: $r"
