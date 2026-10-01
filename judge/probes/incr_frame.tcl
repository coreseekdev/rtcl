proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t a {set x 1a; catch {incr x 1a}; set ::errorInfo}
t b {catch {incr nosuch 1a}; list $::errorInfo [info exists nosuch]}
t c {set x 1; catch {incr x 1a}; set ::errorInfo}
t d {unset -nocomplain y; catch {incr y 08}; set y}
