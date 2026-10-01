proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t a {info functions}
t b {info functions s*}
t c {info functions sin}
t d {info tclversion}
t e {info tclv}
t f {info functions nosuch*}
t g {info locals 1 2}
t h {info procs 2 3}
t i {info vars a b}
t j {info functions abs max min}
