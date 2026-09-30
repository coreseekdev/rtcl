proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t e01 {expr 01}
t e0x10 {expr 0x10}
t e0b101 {expr 0b101}
t e018 {expr 018}
t e09 {expr 09}
t e01plus {expr {01+0}}
t e0x10plus {expr {0x10+0}}
t et {expr {t}}
t ey {expr {y}}
t eyes {expr {yes}}
t eg {expr {g}}
t btru {expr bool(tru)}
t bfx {expr bool(fx)}
t bye {expr bool(ye)}
t bnx {expr bool(nx)}
t bon {expr bool(on)}
t boff {expr bool(off)}
t abs_ws0x0 {::tcl::mathfunc::abs { 	0x0}}
t abs_1e324 {::tcl::mathfunc::abs 1e-324}
t abs_ws1 {::tcl::mathfunc::abs { 1}}
t abs_0x0 {::tcl::mathfunc::abs 0x0}
t abs_1e300 {::tcl::mathfunc::abs 1e-300}
t abs_negws {::tcl::mathfunc::abs { -0x10}}
t esc374alone {expr {"\374"}}
t escxFCalone {expr {"\xFC"}}
t ent_neg1e100 {expr entier(-1e100)}
t int_neg1e100 {expr int(-1e100)}
t round1e100 {expr round(1e100)}
t ent_half {expr entier(-0.5)}
t ent_half_p {expr entier(0.5)}
t bigshl {expr {9223372036854775808 << 1}}
t powdiv {expr {2**63 / 3}}
t negpow {expr {-2**63}}
t subbig {expr {-9223372036854775808 - 1}}
t wideok {expr wide(9223372036854775807)}
t hexadd {expr {0x7fffffffffffffff + 1}}
t bigfloat {expr {9223372036854775808.0}}
t entbigfloat {expr entier(9.223372036854776e18)}
t intws {expr int(" 0x0")}
t floordiv {expr {5 / -3}}
t b_boolnum {expr bool(2)}
t b_boolempty {expr bool()}
t abs_str {::tcl::mathfunc::abs abc}
t int_str {expr int(abc)}
