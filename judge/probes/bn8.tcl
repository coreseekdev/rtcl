proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t zdiv {expr {0.0/0.0}}
t nan_fn {expr nan()}
t inf_fn {expr inf()}
t floathalf {expr {1.5}}
t bigfloat {expr {1e17}}
t smallfloat {expr {1e-5}}
t neg0 {expr {-0.0}}
t pi {expr {acos(-1)}}
