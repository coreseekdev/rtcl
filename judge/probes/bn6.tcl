proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t nan_lower {expr nan}
t NaN_upper {expr NaN}
t int_nan_lower {expr int(nan)}
t nan_plus {expr {nan + 0}}
t abs_12 {expr abs(1,2)}
t abs_0 {expr abs()}
t wide_2args {expr wide(1,2)}
t double_empty {expr double()}
t abs_neg0_str {::tcl::mathfunc::abs "-0.0"}
t abs_nan_str {::tcl::mathfunc::abs "NaN"}
t int_ws_neg {expr int(" -1")}
