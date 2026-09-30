proc t {label script} {
    if {[catch {uplevel 1 $script} r]} { puts "$label ERR: $r" } else { puts "$label: <$r>" }
}
t a {format {%*2$d} 6 42}
t b {format {%1$*d} 6 42}
t c {format {%1$5$d} 42}
t d {format {%*5$d} 6 42}
t e {format {%5$*d} 6 42}
