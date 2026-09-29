set __judge_c [catch {expr 1.3&&3.3} __judge_r]
if {$__judge_c == 0 && $__judge_r eq {1}} {puts PASS} else {puts "FAIL c=$__judge_c r=$__judge_r"}
