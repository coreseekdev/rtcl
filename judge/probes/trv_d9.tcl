proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t e1 {
    unset -nocomplain x
    set info {}
    trace add variable x read q
    proc q {name1 name2 op} {
        global info
        set info [list $name1 $name2 $op]
        global $name1
        set ${name1}($name2) wolf
    }
    proc p {} {
        global x
        set x(X) willi
        return $x(Y)
    }
    list [catch {p} msg] $msg $info
}
