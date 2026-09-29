set __judge_c [catch {
    namespace eval test_ns_2 {
	proc x {} {}
	trace add command x delete "namespace delete [namespace current];#"
	namespace delete [namespace current]
    }
} __judge_r]
if {$__judge_c == 0 && $__judge_r eq {}} {puts PASS} else {puts "FAIL namespace-7.4 c=$__judge_c r=$__judge_r"}
