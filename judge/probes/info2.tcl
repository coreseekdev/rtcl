proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t a {info level}
t b {info level 0}
t c {info level 1}
t d {info level -1}
t e {info level 3}
t f {proc p1 {x} {p2 $x}; proc p2 {y} {list [info level -1] [info level -2] [info level -3]}; p1 zz}
t g {proc p1 {x} {p2 $x}; proc p2 {y} {info level 3}; p1 zz}
t h {proc p {a b c} {info level 0}; p 1 2 3}
t i {proc p {} {info level 1}; p}
t j {proc t1 {a b} {t2 [expr {$a*2}] $b}; proc t2 {x y} {info level 2}; t1 146 z}
t k {proc p {x} {info level 123a}; p 1}
t l {info args}
t m {info args a b c}
t n {info default}
t o {proc t1 args {}; info default t1 args v; list $v [info default t1 a v2]}
t p {info exists a b c}
t q {info commands}
t r {info globals a b}
t s {info level 2 3}
t u {proc p {x} {info level x}; p 1}
