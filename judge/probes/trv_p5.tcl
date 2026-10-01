proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
unset -nocomplain x
trace add variable x read q
proc q {name1 name2 op} {
    global info
    set info [list $name1 $name2 $op]
    global $name1
    set ${name1}($name2) wolf
}
set info {}
t set123 {set x 123}
t exists {info exists x}
t isset {array exists x}
t setXX {set x(X) willi}
t isset2 {array exists x}
t info-now {set info}
