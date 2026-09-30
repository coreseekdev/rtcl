proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t shf {expr {1.5 << 2}}
t shfs {expr {"2.5" >> 1}}
t shs {expr {"abc" << 1}}
t rotf {expr {1.5 <<< 2}}
