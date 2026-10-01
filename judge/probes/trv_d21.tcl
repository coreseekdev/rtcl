namespace eval test_ns_1 {
	proc p {} {
	    namespace delete [namespace current]
	    return [namespace current]
	}
}
puts "1: [test_ns_1::p]"
catch {test_ns_1::p} m
puts "2: $m"

namespace eval test_ns_2 {
	proc p2 {} { namespace delete [namespace current]; return [namespace current] }
}
namespace eval test_ns_2 { puts "3: [p2]" }
puts "4: [namespace exists test_ns_2]"

namespace eval test_ns_3 { namespace delete [namespace current]; puts "5: [namespace current]" }
puts "6: [namespace exists test_ns_3]"
