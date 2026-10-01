proc try {label script} {
    set r [catch {uplevel 1 $script} res]
    puts "$label => c=$r res=$res"
}
unset -nocomplain a
set a(bar) 0
trace add variable a read {unset -nocomplain a(bar) ;#}
try P1elemOnly {array get a}
unset -nocomplain a
set a(bar) 0
trace add variable a read {unset -nocomplain a; set a(bar) 9 ;#}
try P2recreateSame {array get a}
unset -nocomplain a
set a(bar) 0
trace add variable a read {unset -nocomplain a; set a(foo) 1 ;#}
try P3recreateOther {array get a}
unset -nocomplain a
set a(bar) 0
trace add variable a read {unset -nocomplain a ;#}
try P4gone {array get a}
# element value CHANGED by trace:
unset -nocomplain a
set a(bar) 0
trace add variable a read {set a(bar) 5 ;#}
try P5changed {array get a}
# trace ERRORS during array get:
unset -nocomplain a
set a(bar) 0
trace add variable a read {error BOOM ;#}
try P6traceErr {array get a}
# multiple elements with unset-one trace
unset -nocomplain a
set a(p) 1
set a(q) 2
trace add variable a read {unset -nocomplain a(q) ;#}
try P7two {array get a}
