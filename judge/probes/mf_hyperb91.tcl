# mf_hyperb91.tcl — 9.1 C99 全集钉形：双曲/指数族（TIP 745）
# 对 tclsh9.1 实测钉形（.refer/tcl910）；errno 口径按 tclBasic.c 的
# CheckDoubleResult：atanh(±1) 的 ±Inf 接受、atanh 域外 NaN → 域错误、
# asinh/atanh 子正规结果置 ERANGE → "too large to represent"。
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
# acosh：定义域 [1,inf]；-0.0/-inf 也是域错误（glibc NaN → EDOM）
t acosh1 {tcl::mathfunc::acosh 1}
t acosh2 {tcl::mathfunc::acosh 2}
t acosh3 {tcl::mathfunc::acosh -1}
t acosh4 {tcl::mathfunc::acosh 0.5}
t acosh5 {tcl::mathfunc::acosh 0}
t acosh6 {tcl::mathfunc::acosh -0.0}
t acosh7 {tcl::mathfunc::acosh 3.0}
t acosh8 {tcl::mathfunc::acosh 1e300}
t acosh9 {tcl::mathfunc::acosh inf}
t acosh10 {tcl::mathfunc::acosh -inf}
t acosh11 {tcl::mathfunc::acosh}
t acosh12 {tcl::mathfunc::acosh a b}
t acosh13 {tcl::mathfunc::acosh a}
t acosh14 {tcl::mathfunc::acosh 12345678901234567890123456789012345678901234567890}
# asinh：全域；-0.0 保持符号
t asinh1 {tcl::mathfunc::asinh 1}
t asinh2 {tcl::mathfunc::asinh 2}
t asinh3 {tcl::mathfunc::asinh -1}
t asinh4 {tcl::mathfunc::asinh 0.5}
t asinh5 {tcl::mathfunc::asinh 0}
t asinh6 {tcl::mathfunc::asinh -0.0}
t asinh7 {tcl::mathfunc::asinh 3.0}
t asinh8 {tcl::mathfunc::asinh 1e300}
t asinh9 {tcl::mathfunc::asinh inf}
t asinh10 {tcl::mathfunc::asinh -inf}
t asinh11 {tcl::mathfunc::asinh}
t asinh12 {tcl::mathfunc::asinh a b}
t asinh13 {tcl::mathfunc::asinh a}
# 子正规结果：glibc 置 ERANGE → 报 too-large（erfc 同族行为）
t asinh14 {tcl::mathfunc::asinh 1e-310}
# atanh：定义域 (-1,1)；±1 是 ±Inf（ERANGE+Inf 接受）；域外 NaN → 域错误
t atanh1 {tcl::mathfunc::atanh 1}
t atanh2 {tcl::mathfunc::atanh -1}
t atanh3 {tcl::mathfunc::atanh 2}
t atanh4 {tcl::mathfunc::atanh 0.5}
t atanh5 {tcl::mathfunc::atanh 0}
t atanh6 {tcl::mathfunc::atanh -0.0}
t atanh7 {tcl::mathfunc::atanh 3.0}
t atanh8 {tcl::mathfunc::atanh 1e300}
t atanh9 {tcl::mathfunc::atanh inf}
t atanh10 {tcl::mathfunc::atanh -inf}
t atanh11 {tcl::mathfunc::atanh}
t atanh12 {tcl::mathfunc::atanh a b}
t atanh13 {tcl::mathfunc::atanh a}
# 子正规结果：ERANGE → too-large（atanh 的钉形，asinh 同）
t atanh14 {tcl::mathfunc::atanh 1e-310}
# exp2 / expm1：溢出给 Inf（接受），下溢给 0.0（接受）
t exp2_1 {tcl::mathfunc::exp2 1}
t exp2_2 {tcl::mathfunc::exp2 2}
t exp2_3 {tcl::mathfunc::exp2 -1}
t exp2_4 {tcl::mathfunc::exp2 0.5}
t exp2_5 {tcl::mathfunc::exp2 0}
t exp2_6 {tcl::mathfunc::exp2 -0.0}
t exp2_7 {tcl::mathfunc::exp2 3.0}
t exp2_8 {tcl::mathfunc::exp2 1e300}
t exp2_9 {tcl::mathfunc::exp2 inf}
t exp2_10 {tcl::mathfunc::exp2 -inf}
t exp2_11 {tcl::mathfunc::exp2 -1074.1}
t exp2_12 {tcl::mathfunc::exp2 -1074.9}
t exp2_13 {tcl::mathfunc::exp2 -1075.5}
t exp2_14 {tcl::mathfunc::exp2}
t exp2_15 {tcl::mathfunc::exp2 a b}
t exp2_16 {tcl::mathfunc::exp2 a}
t exp2_17 {expr {exp2(2)}}
t exp2_18 {expr {exp2(0.5)}}
t expm1_1 {tcl::mathfunc::expm1 1}
t expm1_2 {tcl::mathfunc::expm1 2}
t expm1_3 {tcl::mathfunc::expm1 -1}
t expm1_4 {tcl::mathfunc::expm1 0.5}
t expm1_5 {tcl::mathfunc::expm1 0}
t expm1_6 {tcl::mathfunc::expm1 -0.0}
t expm1_7 {tcl::mathfunc::expm1 3.0}
t expm1_8 {tcl::mathfunc::expm1 1e300}
t expm1_9 {tcl::mathfunc::expm1 inf}
t expm1_10 {tcl::mathfunc::expm1 -inf}
t expm1_11 {tcl::mathfunc::expm1 -1074.3}
t expm1_12 {tcl::mathfunc::expm1 5e-324}
t expm1_13 {tcl::mathfunc::expm1}
t expm1_14 {tcl::mathfunc::expm1 a b}
t expm1_15 {tcl::mathfunc::expm1 a}
t expm1_16 {expr {expm1(1)}}
