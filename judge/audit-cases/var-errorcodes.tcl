proc E {tag script} {
    set ::errorCode NONE
    set c [catch {uplevel 1 $script} r]
    puts "$tag|$c|$r|$::errorCode"
}
E read-missing {set nope}
E read-missing-q {set ::nope}
E read-array {array set a {1 x}; set a}
E read-elem-missing {array set a {1 x}; set a(9)}
E read-elem-bad {array set a {1 x}; set a([)}
E read-scalar-as-elem {set s 1; set s(1)}
E write-array {array set a {1 x}; set a 2}
E write-elem-on-scalar {set s 1; set s(1) 2}
E write-elem-bad {array set a {1 x}; set a([) 2}
E append-array {array set a {1 x}; append a 2}
E append-elem-scalar {set s 1; append s(1) 2}
E lappend-array {array set a {1 x}; lappend a 2}
E incr-array {array set a {1 x}; incr a}
E incr-scalar-elem {set s 1; incr s(1)}
E unset-missing {unset nope}
E unset-elem-missing {array set a {1 x}; unset a(9)}
proc ra {args} { return -code error "no read" }
set v 1
trace add variable v read ra
E read-trace-fail {set v}
E append-read-trace-fail {append v 2}
E incr-read-trace-fail {incr v}
E lappend-read-trace-fail {lappend v 2}
trace remove variable v read ra
proc wa {args} { return -code error "no write" }
trace add variable v write wa
E write-trace-fail {set v 2}
E append-write-trace-fail {append v 2}
E lappend-write-trace-fail {lappend v 2}
E incr-write-trace-fail {incr v}
trace remove variable v write wa
array set ta {1 x}
trace add variable ta write wa
E arr-write-trace-fail {set ta(1) 2}
E arr-elem-append-trace-fail {append ta(1) 2}
trace add variable ta(1) read ra
E arr-read-trace-fail {set ta(1)}
trace remove variable ta write wa
E unset-trace-fail {unset ta(1)}
trace remove variable ta(1) read ra
proc ua {args} { return -code error "no unset" }
trace add variable ta(1) unset ua
E unset-trace-fail2 {unset ta(1)}
puts DONE
