proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
proc mk {} {
    uplevel 1 {
	set ::log {}
	proc ::tr {a b c} { lappend ::log "$a/$b/$c" }
    }
}
# 1) array exists, element missing
mk; unset -nocomplain x; set x(X) willi
trace add variable x read ::tr
t elem-missing {set x(Y)}
t log1 {set ::log}
# 2) array missing entirely
mk; unset -nocomplain y; trace add variable y read ::tr
t elem-noarray {catch {set y(2)}}
t log2 {set ::log}
# 3) scalar, element read
mk; unset -nocomplain z; set z 5; trace add variable z read ::tr
t elem-scalar {catch {set z(2)} m3}
t scalar-msg {set m3}
t log3 {set ::log}
# 4) info exists on array elem (array exists, elem missing)
mk; unset -nocomplain w; set w(X) 1; trace add variable w read ::tr
t info-exists-elem {info exists w(Y)}
t log4 {set ::log}
# 5) info exists on missing array elem
mk; unset -nocomplain v; trace add variable v read ::tr
t info-exists-noarray {info exists v(2)}
t log5 {set ::log}
# 6) read trace fires on array get? which elements
mk; unset -nocomplain g; set g(a) 1; set g(b) 2; trace add variable g read ::tr
t array-get {array get g}
t log6 {set ::log}
