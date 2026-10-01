catch {namespace delete {*}[namespace children :: test_ns_*]}
catch {rename p ""}
catch {rename q ""}
proc p {} { return "p in [namespace current]" }
proc q {} { return "q in [namespace current]" }
namespace eval test_ns_basic { proc callP {} { p } }
puts "r1=[catch {list [test_ns_basic::callP] [rename q test_ns_basic::p] [test_ns_basic::callP]} m]<$m>"
