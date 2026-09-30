proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t cnt_f {expr {1 << 2.5}}
t cnt_s {expr {1 << "abc"}}
t both_f {expr {1.5 << 2.5}}
t cnt_neg {expr {1 >> -1}}
t cnt_big {expr {1 << 99999999}}
