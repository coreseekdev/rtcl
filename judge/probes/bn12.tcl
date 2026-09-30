proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t bitf {expr {1.5 | 2}}
t bitfs {expr {"2.5" & 3}}
t bits {expr {"abc" ^ 1}}
