# mf_class91.tcl — 9.1 C99 全集钉形：分类函数（TIP 521）+ signbit。
# 对 tclsh9.1 实测。要点：
# - 分类函数走 GetNumberFromObj 口径："nan"/"inf"/"NaN" 串是数（NaN
#   类），非数字串报 `expected number but got`（无 floating-point 字
#   样，与普通数学函数的分界）；bignum 转 double 可溢出 → isinf 是 1；
# - isfinite 的 NaN 是 0（dCls != INFINITE && != NAN 口径）；
# - signbit：整数负值/bignum 负值按值号；-0.0 是 1；int 的 -0 是 0；
#   fpclassify 同口径，五词：zero/subnormal/normal/infinite/nan。
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t isnan1 {tcl::mathfunc::isnan nan}
t isnan2 {tcl::mathfunc::isnan NaN}
t isnan3 {tcl::mathfunc::isnan 1}
t isnan4 {tcl::mathfunc::isnan -inf}
t isnan5 {tcl::mathfunc::isnan 1e-320}
t isnan6 {tcl::mathfunc::isnan 0x10}
t isnan7 {tcl::mathfunc::isnan}
t isnan8 {tcl::mathfunc::isnan a b}
t isnan9 {tcl::mathfunc::isnan abc}
t isnan10 {expr {isnan(1)}}
t isinf1 {tcl::mathfunc::isinf inf}
t isinf2 {tcl::mathfunc::isinf -inf}
t isinf3 {tcl::mathfunc::isinf Infinity}
t isinf4 {tcl::mathfunc::isinf 1}
t isinf5 {tcl::mathfunc::isinf nan}
t isinf6 {tcl::mathfunc::isinf 1e-320}
t isinf7 {tcl::mathfunc::isinf 1e309}
t isinf8 {tcl::mathfunc::isinf 1234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890}
t isinf9 {tcl::mathfunc::isinf}
t isinf10 {tcl::mathfunc::isinf 1 2}
t isinf11 {tcl::mathfunc::isinf abc}
t isinf12 {expr {isinf(1)}}
t isfinite1 {tcl::mathfunc::isfinite 1}
t isfinite2 {tcl::mathfunc::isfinite inf}
t isfinite3 {tcl::mathfunc::isfinite nan}
t isfinite4 {tcl::mathfunc::isfinite -inf}
t isfinite5 {tcl::mathfunc::isfinite 0}
t isfinite6 {tcl::mathfunc::isfinite 1e-320}
t isfinite7 {expr {isfinite(1)}}
t isnormal1 {tcl::mathfunc::isnormal 0}
t isnormal2 {tcl::mathfunc::isnormal 1e-320}
t isnormal3 {tcl::mathfunc::isnormal 1e-310}
t isnormal4 {tcl::mathfunc::isnormal 1e300}
t isnormal5 {tcl::mathfunc::isnormal 2.2250738585072014e-308}
t isnormal6 {tcl::mathfunc::isnormal inf}
t isnormal7 {tcl::mathfunc::isnormal nan}
t isnormal8 {expr {isnormal(1)}}
t issub1 {tcl::mathfunc::issubnormal 1e-320}
t issub2 {tcl::mathfunc::issubnormal 1e-3200}
t issub3 {tcl::mathfunc::issubnormal 0}
t issub4 {tcl::mathfunc::issubnormal 2.2250738585072014e-308}
t issub5 {tcl::mathfunc::issubnormal 2.225073858507201e-308}
t issub6 {tcl::mathfunc::issubnormal 5e-324}
t issub7 {expr {issubnormal(1e-320)}}
t isunord1 {tcl::mathfunc::isunordered nan 1}
t isunord2 {tcl::mathfunc::isunordered 1 nan}
t isunord3 {tcl::mathfunc::isunordered 1 2}
t isunord4 {tcl::mathfunc::isunordered nan nan}
t isunord5 {tcl::mathfunc::isunordered inf -inf}
t isunord6 {tcl::mathfunc::isunordered 1}
t isunord7 {tcl::mathfunc::isunordered 1 2 3}
t isunord8 {tcl::mathfunc::isunordered a b}
t isunord9 {expr {isunordered(1,2)}}
t signbit1 {tcl::mathfunc::signbit 123.456}
t signbit2 {tcl::mathfunc::signbit -0.0}
t signbit3 {tcl::mathfunc::signbit -inf}
t signbit4 {tcl::mathfunc::signbit 0}
t signbit5 {tcl::mathfunc::signbit 123}
t signbit6 {tcl::mathfunc::signbit -12}
t signbit7 {tcl::mathfunc::signbit 1231231231231231231231231231231231231231231231231223}
t signbit8 {tcl::mathfunc::signbit -12345678901234567890123456789012345678901234567890}
t signbit9 {tcl::mathfunc::signbit " 5"}
t signbit10 {tcl::mathfunc::signbit " -5"}
t signbit11 {tcl::mathfunc::signbit -0}
t signbit12 {tcl::mathfunc::signbit 0x10}
t signbit13 {tcl::mathfunc::signbit 0b101}
t signbit14 {tcl::mathfunc::signbit 1e-320}
t signbit15 {tcl::mathfunc::signbit nan}
t signbit16 {tcl::mathfunc::signbit true}
t signbit17 {tcl::mathfunc::signbit}
t signbit18 {tcl::mathfunc::signbit 1 2}
t signbit19 {expr {signbit(-12)}}
t fp1 {fpclassify [expr {0.0/0.0}]}
t fp2 {fpclassify nan}
t fp3 {fpclassify NaN}
t fp4 {fpclassify inf}
t fp5 {fpclassify -inf}
t fp6 {fpclassify 0}
t fp7 {fpclassify -0.0}
t fp8 {fpclassify 1e-320}
t fp9 {fpclassify 2.5}
t fp10 {fpclassify 0x10}
t fp11 {fpclassify 0b101}
t fp12 {fpclassify 1e-3200}
t fp13 {fpclassify 1e309}
t fp14 {fpclassify abc}
t fp15 {fpclassify true}
t fp16 {fpclassify " 2.5 "}
t fp17 {fpclassify +2.5}
t fp18 {fpclassify 1234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890}
t fp19 {fpclassify}
t fp20 {fpclassify a b}
