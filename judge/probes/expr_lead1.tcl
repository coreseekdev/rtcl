proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t a {expr 028.1 + 09.2}
t b {expr 0289.1}
t c {expr -~3}
t d {expr ~~3}
t e {expr - - 3}
t f {expr +-3}
t g {expr 08.5}
t h {expr 09.2e1}
t i {expr 0281}
t j {expr 0x1.8p3}
