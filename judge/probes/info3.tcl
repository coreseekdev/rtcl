proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t a {namespace eval x {info level}}
t b {namespace eval x {proc pp {q} {info level 1}}; pp yy}
t c {namespace eval x {info level 1}}
t d {info def}
t e {info defau}
t f {proc t1 {a {b bb}} {}; set r [info default t1 a v]; list $r $v}
t g {proc t1 {a {b bb}} {}; set r [info default t1 b v]; list $r $v}
t h {proc t1 args {}; set r [info default t1 args v]; list $r $v}
t i {proc t1 {a} {}; catch {info default t1 a}; set ::ec $errorCode; set ec}
t j {proc t1 {a} {}; catch {info default nosuch a v}; set ::ec $errorCode; set ec}
t k {proc t1 {a} {}; catch {info default t1 zz v}; set ::ec $errorCode; set ec}
t l {proc t1 {a} {}; set r [info default t1 a v 4]; set r}
