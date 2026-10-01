proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
unset -nocomplain x
t exists-after-trace {trace add variable x write T; info exists x}
t info-after-trace {trace info variable x}
t unset-missing {catch {unset x}}
t info-after-unset {trace info variable x}
unset -nocomplain y
trace add variable y write T
t unset-nocomplain-missing {unset -nocomplain y}
t info-after-nc-unset {trace info variable y}
# array phantom state
unset -nocomplain a
trace add variable a(0) write T
t array-exists {info exists a}
t array-names {array names a}
t elem-exists {info exists a(0)}
unset -nocomplain a
t array-exists2 {info exists a}
t elem-info2 {trace info variable a(0)}
# scalar 44 then element trace
unset -nocomplain s
set s 44
t trace-elem-scalar {catch {trace add variable s(0) write T} m}
t trace-elem-scalar-msg {set m}
# trace add on missing element of real array
unset -nocomplain b
set b(1) 2
trace add variable b(9) read T
t b-names {array names b}
t b-size {array size b}
t b-exists9 {info exists b(9)}
t b-read9 {catch {set b(9)} m2}
