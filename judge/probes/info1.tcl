proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t a {info args set 1}
t b {info commands a b}
t c {info complete}
t d {info default t1 a v}
t e {info default a b}
t f {info default _nonexistent_ a b}
t g {proc t1 {a b} {}; info default t1 x v}
t h {info exists}
t i {info exists 1 2}
t j {info globals 1 2}
t k {info level 1 2}
t l {info level 123a}
t m {proc t1 {a b {c d} {e x}} {}; info default t1 a vv; list $vv}
t n {proc t1 {a b {c d}} {}; info default t1 c vv; list $vv}
t o {info default set args vv}
t p {proc t1 {a b} {t2 [expr {$a*2}] $b}; proc t2 {x y} {list [info level] [info level 1] [info level 2] [info level -1] [info level 0]}; t1 146 {a {b c}}}
t q {info level 0}
