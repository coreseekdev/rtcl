proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t fdiv00 {expr {0.0/0.0}}
t fdiv10 {expr {1.0/0.0}}
t fdivneg {expr {-1.0/0.0}}
t idiv0 {expr {1/0}}
t fmod0 {expr {1.0 % 0.0}}
t fand0 {expr {1.5 && 0.0}}
