proc t {label script} {
    if {[catch {uplevel 1 $script} r]} { puts "$label ERR: $r" } else { puts "$label: <$r>" }
}
t ws {format %d { 12}}
t ws2 {format %d {12 }}
t us {format %d 1_000}
t us2 {format %d 0x_1}
t ob {format %d 0b101}
t oo {format %d 0o17}
t hex.neg {format %d -0x10}
t oct.lead {format %d 017}
t bad.oct {format %d 08}
t bad.oct2 {format %d 0o18}
t i.of.intval {set x [expr {5}]; format %d $x}
t wide.of.str {format %d 9223372036854775808}
t wide.str2 {format %d -9223372036854775809}
