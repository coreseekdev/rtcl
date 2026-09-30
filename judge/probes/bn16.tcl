proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t neg0 {::tcl::mathfunc::abs -0}
t neghex0 {::tcl::mathfunc::abs -0x0}
t plus0 {::tcl::mathfunc::abs +0}
t plus5 {::tcl::mathfunc::abs +5}
t zerozero {::tcl::mathfunc::abs 00}
t neg00 {::tcl::mathfunc::abs -00}
t neg0e0 {::tcl::mathfunc::abs -0.0e0}
t negbig {::tcl::mathfunc::abs -18446744073709551617}
t ws5 {::tcl::mathfunc::abs " 5"}
