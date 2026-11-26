# mf_log91.tcl — 9.1 C99 全集钉形：对数/幂族 + gamma（TIP 745）
# 对 tclsh9.1 实测钉形。要点：
# - log1p/log2 域外（<-1 / <=0）glibc 返回 NaN → 域错误；log1p(-1)
#   与 log2(0) 的 -Inf 是 ERANGE+Inf，接受；
# - gamma 就是 tgamma（gamma(-0.5) 是负数 — 手册"absolute value"
#   表述与 BuiltinFuncTable 的 tgamma 映射不符，实现为准）；极点
#   （非正整数）NaN → 域错误，±0 → ±Inf（ERANGE 接受），溢出 → Inf；
# - lgamma 极点给 +Inf，负无穷也给 +Inf；溢出（1e300）不发生
#   （lgamma 是对数域，结果仍 normal）。
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t log1p1 {tcl::mathfunc::log1p 1}
t log1p2 {tcl::mathfunc::log1p 2}
t log1p3 {tcl::mathfunc::log1p -1}
t log1p4 {tcl::mathfunc::log1p 0.5}
t log1p5 {tcl::mathfunc::log1p 0}
t log1p6 {tcl::mathfunc::log1p -0.0}
t log1p7 {tcl::mathfunc::log1p 3.0}
t log1p8 {tcl::mathfunc::log1p 1e300}
t log1p9 {tcl::mathfunc::log1p inf}
t log1p10 {tcl::mathfunc::log1p -inf}
t log1p11 {tcl::mathfunc::log1p -4}
t log1p12 {tcl::mathfunc::log1p 1e-320}
t log1p13 {tcl::mathfunc::log1p}
t log1p14 {tcl::mathfunc::log1p a b}
t log1p15 {tcl::mathfunc::log1p a}
t log1p16 {expr {log1p(1)}}
t log2_1 {tcl::mathfunc::log2 1}
t log2_2 {tcl::mathfunc::log2 2}
t log2_3 {tcl::mathfunc::log2 -1}
t log2_4 {tcl::mathfunc::log2 0.5}
t log2_5 {tcl::mathfunc::log2 0}
t log2_6 {tcl::mathfunc::log2 -0.0}
t log2_7 {tcl::mathfunc::log2 3.0}
t log2_8 {tcl::mathfunc::log2 1e300}
t log2_9 {tcl::mathfunc::log2 inf}
t log2_10 {tcl::mathfunc::log2 -inf}
t log2_11 {tcl::mathfunc::log2 -4}
t log2_12 {tcl::mathfunc::log2 8}
t log2_13 {tcl::mathfunc::log2}
t log2_14 {tcl::mathfunc::log2 a b}
t log2_15 {tcl::mathfunc::log2 a}
t log2_16 {expr {log2(8)}}
# logb：整数化指数；0/-0.0 的 -Inf 是 ERANGE 接受
t logb1 {tcl::mathfunc::logb 1}
t logb2 {tcl::mathfunc::logb 2}
t logb3 {tcl::mathfunc::logb -1}
t logb4 {tcl::mathfunc::logb 0.5}
t logb5 {tcl::mathfunc::logb 0}
t logb6 {tcl::mathfunc::logb -0.0}
t logb7 {tcl::mathfunc::logb 3.0}
t logb8 {tcl::mathfunc::logb 1e300}
t logb9 {tcl::mathfunc::logb inf}
t logb10 {tcl::mathfunc::logb -inf}
t logb11 {tcl::mathfunc::logb 5e-324}
t logb12 {tcl::mathfunc::logb 1e-310}
t logb13 {tcl::mathfunc::logb}
t logb14 {tcl::mathfunc::logb a b}
t logb15 {tcl::mathfunc::logb a}
t logb16 {tcl::mathfunc::logb 9}
t logb17 {tcl::mathfunc::logb -4}
# gamma = tgamma
t gamma1 {tcl::mathfunc::gamma 1}
t gamma2 {tcl::mathfunc::gamma 5}
t gamma3 {tcl::mathfunc::gamma 0.5}
t gamma4 {tcl::mathfunc::gamma -0.5}
t gamma5 {tcl::mathfunc::gamma 0}
t gamma6 {tcl::mathfunc::gamma -0.0}
t gamma7 {tcl::mathfunc::gamma -1}
t gamma8 {tcl::mathfunc::gamma -2}
t gamma9 {tcl::mathfunc::gamma 171.6}
t gamma10 {tcl::mathfunc::gamma 171.63}
t gamma11 {tcl::mathfunc::gamma 200}
t gamma12 {tcl::mathfunc::gamma inf}
t gamma13 {tcl::mathfunc::gamma 1e-320}
t gamma14 {tcl::mathfunc::gamma -1e-320}
t gamma15 {tcl::mathfunc::gamma 2.5}
t gamma16 {tcl::mathfunc::gamma}
t gamma17 {tcl::mathfunc::gamma a b}
t gamma18 {tcl::mathfunc::gamma a}
t gamma19 {expr {gamma(5)}}
t lgamma1 {tcl::mathfunc::lgamma 1}
t lgamma2 {tcl::mathfunc::lgamma 0.5}
t lgamma3 {tcl::mathfunc::lgamma -0.5}
t lgamma4 {tcl::mathfunc::lgamma 0}
t lgamma5 {tcl::mathfunc::lgamma -1}
t lgamma6 {tcl::mathfunc::lgamma -2}
t lgamma7 {tcl::mathfunc::lgamma 1e300}
t lgamma8 {tcl::mathfunc::lgamma inf}
t lgamma9 {tcl::mathfunc::lgamma -inf}
t lgamma10 {tcl::mathfunc::lgamma 1e-320}
t lgamma11 {tcl::mathfunc::lgamma 2.5}
t lgamma12 {tcl::mathfunc::lgamma}
t lgamma13 {tcl::mathfunc::lgamma a b}
t lgamma14 {tcl::mathfunc::lgamma a}
t lgamma15 {expr {lgamma(2.5)}}
