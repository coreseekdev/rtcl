proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
unset -nocomplain x
set info {}
proc q {name1 name2 op} {
    global info
    set info [list $name1 $name2 $op]
    global $name1
    set ${name1}($name2) wolf
}
trace add variable x read q
t set-scalar {set x 123}
t x-is-array {info exists x}
proc p {} {
    global x
    t set-elem-on-scalar {set x(X) willi}
    return $x(Y)
}
t call-p {p}
t info {set info}
t array-get {array get x}
