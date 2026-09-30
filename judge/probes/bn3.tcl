proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t bool_o {expr bool(o)}
t bool_of {expr bool(of)}
t bool_ox {expr bool(ox)}
t bool_fa {expr bool(fa)}
t bool_na {expr bool(na)}
t bool_TRUE {expr bool(TRUE)}
t bool_Yes {expr bool(Yes)}
t bool_2_5 {expr bool(2.5)}
t bool_0x10 {expr bool(0x10)}
t bool_neg1 {expr bool(-1)}
t bool_0_0 {expr bool(0.0)}
t bool_00 {expr bool(00)}
t int_Inf {expr int(Inf)}
t int_NegInf {expr int(-Inf)}
t int_NaN {expr int(NaN)}
t wide_Inf {expr wide(Inf)}
t round_Inf {expr round(Inf)}
t round_NegInf {expr round(-Inf)}
t round_neghalf {expr round(-0.5)}
t round_2_5 {expr round(2.5)}
t isqrt_2_5 {expr isqrt(2.5)}
t isqrt_0 {expr isqrt(0)}
t isqrt_1e18 {expr isqrt(1e18)}
t abs_neg1e324 {::tcl::mathfunc::abs -1e-324}
t abs_neg1e300 {::tcl::mathfunc::abs -1e300}
t abs_NaN {::tcl::mathfunc::abs NaN}
t abs_neg0 {::tcl::mathfunc::abs -0.0}
t e017 {expr 017}
t e017plus {expr {017+0}}
t hexeq {expr {0x10 eq 16}}
t bineq {expr {0b101 eq 5}}
t ent_2_5 {expr entier(2.5)}
t wide_neg1_5 {expr wide(-1.5)}
t round_9_4e18 {expr round(9.4e18)}
t ent_1e308 {expr entier(1e308)}
t int_1e308 {expr int(1e308)}
t negeqx {expr {-0x10 eq -16}}
t isqrt_big {expr {isqrt(123456789012345678901234567890)}}
t bool_negbig {expr bool(-9223372036854775809)}
t wide_min {expr wide(-9223372036854775809)}
t not_ws {expr !" 1"}
t and_ws {expr {" 1" && 1}}
