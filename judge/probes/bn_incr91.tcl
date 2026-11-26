# bn_incr91 — D5 重钉（Tcl 9.1）：incr 的任意精度默认——bignum 现值/
# bignum 增量精确求和（不回绕不报错）、9.1 errorCode 分工（非数字 =
# TCL VALUE NUMBER；浮点形 = TCL VALUE INTEGER）。incr_widen 钉 i64
# 溢出双向提升；本文件补 bignum 现值维与 EC 分工维。
# 9.1.0 oracle 逐字节钉形（2026-11-26）；vm/tree 两引擎一致（sweep）。
if {![info exists ::errorCode]} {set ::errorCode NONE}
proc p {label body} { set c [catch {uplevel 1 $body} r]; puts "$label|$c|$r|$::errorCode" }
# bignum 现值：字符串字面量与 expr 结果两种形态
p incr_big_str {set y 10000000000000000000000; incr y; set y}
p incr_big_var {set y [expr {2**63}]; incr y; set y}
p incr_big_down {set y [expr {2**63}]; incr y -1; set y}
p incr_big_amtx {set q 5; incr q 99999999999999999999999; set q}
p incr_big_amtexpr {set q2 5; incr q2 [expr 2**70]; set q2}
p incr_big_arr {set a(k) [expr {2**63}]; incr a(k); set a(k)}
p incr_big_neg {set z [expr {-2**63}]; incr z; set z}
# i64 边界（incr_widen 同族复核）
p incr_up {set y 9223372036854775807; incr y; set y}
p incr_down {set z -9223372036854775808; incr z -1; set z}
p incr_up2 {set y 9223372036854775808; incr y; set y}
p incr_down2 {set z -9223372036854775809; incr z -1; set z}
# 9.1 errorCode 分工：非数字 = TCL VALUE NUMBER
p ec_badcur {set x abc; incr x}
p ec_badamt {set x 12; incr x 3z}
p ec_badboth {catch {incr qq 0x} e; list $e}
# 浮点形现值/增量 = TCL VALUE INTEGER（GetWideInt 失败的老分工保留）
p ec_floatcur {set x 1.5; incr x}
p ec_floatamt {set x 5; incr x 1.5}
# 前导空白与十六进制形态照常收窄
p incr_ws {set q5 { 5}; incr q5; set q5}
p incr_hex {set q6 0x10; incr q6; set q6}
# while 条件里的 bignum 比较（i63 立即数路径外沿）
p while_big {set i [expr 2**62]; set n 0; while {$i > [expr 2**62 - 3]} {incr n; incr i -1}; set n}
