proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t nan_fn {expr nan()}
t q_nan {expr {"nan"}}
t q_nan_plus {expr {"nan" + 0}}
t nan_mul {expr {nan * 2}}
t nanfn_plus {expr {nan() + 0}}
t nan_eq {expr {nan eq nan}}
t nan_dbl {expr double(nan)}
t q_NaN_eq {expr {"NaN" eq "NaN"}}
t nan_div {expr {nan / 1}}
t inf_bare {expr inf}
t inf_plus {expr {inf + 0}}
