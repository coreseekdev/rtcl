proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t shl63 {expr {1 << 63}}
t shl62x4 {expr {4611686018427387904 << 2}}
t shr64neg {expr {-1 >> 64}}
t shr1 {expr {-2 >> 1}}
t shr64pos {expr {1 >> 64}}
t shr65neg {expr {-1 >> 65}}
t shr63 {expr {-1 >> 63}}
t hexmax {expr {0x8000000000000000}}
t abs_expr_neg0 {expr {abs(-0.0)}}
t int_neg0 {expr int(-0.0)}
t round_neg04 {expr round(-0.4)}
t ent_big {expr entier(9223372036854775808)}
t hexeqraw {expr {0x10 eq 0x10}}
t isqrt_4g {expr isqrt(4294967296)}
t ent_neg1e30 {expr entier(-1e30)}
t octeq {expr {017 eq 15}}
t octplus {expr {"017" + 0}}
t bool_sp {expr bool(" ")}
t big_and {expr {-2 & 0xff}}
t big_shr1 {expr {-9223372036854775808 >> 1}}
t big_shl1 {expr {-9223372036854775808 << 1}}
t pow_neg_exp {expr {2**-63}}
t div_big_mix {expr {18446744073709551616 / -3}}
t mod_big_mix {expr {18446744073709551616 % -3}}
