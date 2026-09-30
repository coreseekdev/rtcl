proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t mod_f1 {expr {1.5 % 2}}
t mod_f2 {expr {5 % 2.0}}
t mod_bool {expr {true % 2}}
t add_bool {expr {true + 1}}
t add_yes {expr {yes + 1}}
t add_t {expr {t + 1}}
t bit_bool {expr {true | 2}}
t div00 {expr {0/0.0}}
t div50 {expr {5/0.0}}
t divneg0 {expr {1.0/-0.0}}
t div00neg {expr {-0.0/0.0}}
t cmp_bool {expr {true == 1}}
t int_bool {expr {int(true)}}
