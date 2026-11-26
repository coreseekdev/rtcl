# mf_erffma91.tcl — 9.1 C99 全集钉形：erf/erfc/fma/cbrt/copysign/dim/
# nextafter/remainder/trunc（TIP 745）。对 tclsh9.1 实测。要点：
# - erfc 的子正规结果（erfc 27）置 ERANGE → "too large to represent"；
#   erf 的子正规结果（erf 1e-310）不置 → 原样接受；
# - nextafter 的子正规结果置 ERANGE（nextafter(DBL_MIN,0) 报错），
#   下溢到 0.0 反而接受；nextafter(+0,-0) 是 -0.0；
# - remainder 是 IEEE 余数（商 round-to-even）；x/0 与 x/inf 域错误；
# - fma 溢出给 Inf（接受）；cbrt 超小输入直接下溢 0.0（接受）。
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t erf1 {tcl::mathfunc::erf 1}
t erf2 {tcl::mathfunc::erf 0.5}
t erf3 {tcl::mathfunc::erf -1}
t erf4 {tcl::mathfunc::erf 25}
t erf5 {tcl::mathfunc::erf inf}
t erf6 {tcl::mathfunc::erf -inf}
t erf7 {tcl::mathfunc::erf 1e-310}
t erf8 {tcl::mathfunc::erf}
t erf9 {tcl::mathfunc::erf a b}
t erf10 {tcl::mathfunc::erf a}
t erf11 {expr {erf(0.5)}}
t erfc1 {tcl::mathfunc::erfc 1}
t erfc2 {tcl::mathfunc::erfc 0.5}
t erfc3 {tcl::mathfunc::erfc 26.4}
t erfc4 {tcl::mathfunc::erfc 27}
t erfc5 {tcl::mathfunc::erfc 30}
t erfc6 {tcl::mathfunc::erfc inf}
t erfc7 {tcl::mathfunc::erfc -inf}
t erfc8 {tcl::mathfunc::erfc}
t erfc9 {tcl::mathfunc::erfc a b}
t erfc10 {tcl::mathfunc::erfc a}
t erfc11 {expr {erfc(1)}}
t fma1 {tcl::mathfunc::fma 2 3 5}
t fma2 {tcl::mathfunc::fma 1e308 10 1}
t fma3 {tcl::mathfunc::fma 1e300 1e300 1}
t fma4 {tcl::mathfunc::fma 1e-160 1e-160 -2e-322}
t fma5 {tcl::mathfunc::fma 2 3 -5}
t fma6 {tcl::mathfunc::fma}
t fma7 {tcl::mathfunc::fma a b c d}
t fma8 {tcl::mathfunc::fma 1 2}
t fma9 {tcl::mathfunc::fma a b c}
t fma10 {expr {fma(2,3,5)}}
t cbrt1 {tcl::mathfunc::cbrt 1}
t cbrt2 {tcl::mathfunc::cbrt 2}
t cbrt3 {tcl::mathfunc::cbrt -1}
t cbrt4 {tcl::mathfunc::cbrt 0.5}
t cbrt5 {tcl::mathfunc::cbrt 0}
t cbrt6 {tcl::mathfunc::cbrt -0.0}
t cbrt7 {tcl::mathfunc::cbrt 3.0}
t cbrt8 {tcl::mathfunc::cbrt 1e300}
t cbrt9 {tcl::mathfunc::cbrt inf}
t cbrt10 {tcl::mathfunc::cbrt -inf}
t cbrt11 {tcl::mathfunc::cbrt -8}
t cbrt12 {tcl::mathfunc::cbrt 1e-920}
t cbrt13 {tcl::mathfunc::cbrt 1e-930}
t cbrt14 {tcl::mathfunc::cbrt 0x10}
t cbrt15 {tcl::mathfunc::cbrt 0b101}
t cbrt16 {tcl::mathfunc::cbrt}
t cbrt17 {tcl::mathfunc::cbrt a b}
t cbrt18 {tcl::mathfunc::cbrt abc}
t cbrt19 {expr {cbrt(2)}}
t copysign1 {tcl::mathfunc::copysign 0 -1}
t copysign2 {tcl::mathfunc::copysign 1 -0.0}
t copysign3 {tcl::mathfunc::copysign -0.0 1}
t copysign4 {tcl::mathfunc::copysign 1.5 -3}
t copysign5 {tcl::mathfunc::copysign inf -1}
t copysign6 {tcl::mathfunc::copysign 5e-324 -1}
t copysign7 {tcl::mathfunc::copysign}
t copysign8 {tcl::mathfunc::copysign a b c}
t copysign9 {tcl::mathfunc::copysign 1 2}
t copysign10 {expr {copysign(3,-1)}}
t dim1 {tcl::mathfunc::dim 2 1}
t dim2 {tcl::mathfunc::dim 1 2}
t dim3 {tcl::mathfunc::dim -1 -2}
t dim4 {tcl::mathfunc::dim 1 1}
t dim5 {tcl::mathfunc::dim 2.5 1.5}
t dim6 {tcl::mathfunc::dim -0.0 -0.0}
t dim7 {tcl::mathfunc::dim 1e-320 2e-320}
t dim8 {tcl::mathfunc::dim 1e-320 5e-324}
t dim9 {tcl::mathfunc::dim 1.7e308 -1.7e308}
t dim10 {tcl::mathfunc::dim}
t dim11 {tcl::mathfunc::dim a b c}
t dim12 {expr {dim(2,1)}}
t nextafter1 {tcl::mathfunc::nextafter 123 inf}
t nextafter2 {tcl::mathfunc::nextafter 1 0.5}
t nextafter3 {tcl::mathfunc::nextafter 1e300 -1e300}
t nextafter4 {tcl::mathfunc::nextafter -5e-324 -0.0}
t nextafter5 {tcl::mathfunc::nextafter 0.0 0.0}
t nextafter6 {tcl::mathfunc::nextafter 0.0 -0.0}
t nextafter7 {tcl::mathfunc::nextafter 1 1}
t nextafter8 {tcl::mathfunc::nextafter 5e-324 0}
t nextafter9 {tcl::mathfunc::nextafter 2.225073858507201e-308 0}
t nextafter10 {tcl::mathfunc::nextafter}
t nextafter11 {tcl::mathfunc::nextafter a b c}
t nextafter12 {tcl::mathfunc::nextafter 1 2}
t nextafter13 {expr {nextafter(1,2)}}
t remainder1 {tcl::mathfunc::remainder 12.5 1.5}
t remainder2 {tcl::mathfunc::remainder 12.5 -1.5}
t remainder3 {tcl::mathfunc::remainder -12.5 1.5}
t remainder4 {tcl::mathfunc::remainder 5 2}
t remainder5 {tcl::mathfunc::remainder 1.5 1}
t remainder6 {tcl::mathfunc::remainder 2.5 2}
t remainder7 {tcl::mathfunc::remainder 4.5 2}
t remainder8 {tcl::mathfunc::remainder 1 0}
t remainder9 {tcl::mathfunc::remainder inf 1}
t remainder10 {tcl::mathfunc::remainder 1 inf}
t remainder11 {tcl::mathfunc::remainder 0 0}
t remainder12 {tcl::mathfunc::remainder 3e-320 2e-320}
t remainder13 {tcl::mathfunc::remainder}
t remainder14 {tcl::mathfunc::remainder a b c}
t remainder15 {tcl::mathfunc::remainder 1 2}
t remainder16 {expr {remainder(12.5,1.5)}}
t trunc1 {tcl::mathfunc::trunc 12.5}
t trunc2 {tcl::mathfunc::trunc -12.5}
t trunc3 {tcl::mathfunc::trunc 0.5}
t trunc4 {tcl::mathfunc::trunc -0.0}
t trunc5 {tcl::mathfunc::trunc 1e300}
t trunc6 {tcl::mathfunc::trunc inf}
t trunc7 {tcl::mathfunc::trunc -inf}
t trunc8 {tcl::mathfunc::trunc 12345678901234567890123456789012345678901234567890}
t trunc9 {tcl::mathfunc::trunc}
t trunc10 {tcl::mathfunc::trunc a b}
t trunc11 {tcl::mathfunc::trunc a}
t trunc12 {expr {trunc(-12.5)}}
