# bn_int91 — D5 重钉（Tcl 9.1）：int() 不再截 word（8.6 的低 64 位回绕
# 退役），entier() 同为精确截断，wide() 独自保留低 64 位折算。
# 9.1.0 oracle 逐字节钉形（2026-11-26）；vm/tree 两引擎一致（sweep）。
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
# int()：超界值不再回绕
t int63 {expr int(9223372036854775808)}
t int63neg {expr int(-9223372036854775809)}
t int22 {expr int(1e+22)}
t int22neg {expr int(-1e+22)}
t intbig {expr int(2**100)}
t intbig2 {expr int(123456789012345678901234567890)}
t intnegbig2 {expr int(-123456789012345678901234567890)}
t intstr {expr int("123456789012345678901234567890")}
t int_of_int {expr int(int(2**100))}
# int()：截断朝零、i64 内不变
t intd {expr int(-1.5)}
t int_trunc0 {expr int(-0.5)}
t int_inrange {expr int(5)}
t int_float {expr int(1e18)}
# entier()：同为精确（与 int() 在 9.1 同义）
t entier22 {expr entier(1e+22)}
t entier100 {expr entier(2**100)}
t entier_neg {expr entier(-1.5)}
# wide()：低 64 位折算（补码），9.1 唯一保留回绕的整数转换
t widebig {expr wide(9223372036854775808)}
t wide_neg {expr wide(-9223372036854775809)}
t wide22 {expr wide(1e22)}
t wide_of_big {expr wide(2**100)}
t widepos {expr wide(1.5)}
# 与整族算术的衔接：int() 结果继续参与 bignum 算术
t int_then_add {expr {int(2**100) + 0}}
t int_then_cmp {expr {int(2**63) > 5}}
t int_then_mod {expr {int(2**64) % 1000}}
t int_then_shr {expr {int(2**80) >> 60}}
t int_then_bnot {expr ~int(2**70)}
