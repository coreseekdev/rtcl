proc t {label script} {
    if {[catch {uplevel 1 $script} r]} { puts "$label ERR: $r" } else { puts "$label: <$r>" }
}
t a {format {%*5d} 6 42}
t b {format {%*05d} 6 42}
t c {format {%*d} 6 42}
