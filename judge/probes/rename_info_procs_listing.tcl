catch {rename q ""}
proc q {} { return "q" }
rename q test_ns_basic::p
puts "A=[lsort [info procs ::test_ns_basic::*]]"
puts "B=[lsort [info procs test_ns_basic::*]]"
puts "C=[lsort [info procs *test_ns_basic*]]"
puts "D=[namespace which -command test_ns_basic::p]"
namespace eval test_ns_basic { puts "E=[namespace which -command p]" }
