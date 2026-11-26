# bn_pow91 — D5 重钉（Tcl 9.1）：** 大指数族——负底数奇偶、十进制/
# 十六进制大底、bignum 上下文衔接、错误面。bnprobe2 已钉 2**63..2**100
# 与 3**268435456（exponent too large）；本文件补底数形态维。
# 9.1.0 oracle 逐字节钉形（2026-11-26）；vm/tree 两引擎一致（sweep）。
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t pow_negbase_odd {expr {(-2)**101}}
t pow_negbase_even {expr {(-2)**100}}
t pow_negbase_big {expr {(-3)**41}}
t pow_10_30 {expr {10**30}}
t pow_3_40 {expr {3**40}}
t pow_hexbase {expr {0x10**20}}
t pow_bignum_base {expr {(2**64)**2}}
t pow_bignum_exp {expr {2**(2**62 - 1) > 0}}
t pow_negexp {expr {2**-1}}
t pow_zero {expr {0**0}}
t pow_zero_pos {expr {0**5}}
t pow_one_bigexp {expr {1**268435456}}
t pow_negone_even {expr {(-1)**268435456}}
t pow_negone_odd {expr {(-1)**268435457}}
t pow_i64min {expr {(-9223372036854775808)**2}}
t pow_i64min3 {expr {(-9223372036854775808)**3}}
t pow_then_mod {expr {(2**70) % (2**20)}}
t pow_then_div {expr {(10**30) / (10**15)}}
t pow_then_bits {expr {(2**70) & 0xffff}}
t pow_too_big {expr {2**268435456}}
t pow_neg_too_big {expr {(-2)**268435456}}
t pow_zero_negexp {expr {0**-1}}
t pow_float_exp {expr {2**63 * 1.0}}
t pow_mixed {expr {2**64 == 18446744073709551616}}
