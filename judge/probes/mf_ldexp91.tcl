# mf_ldexp91.tcl — 9.1 C99 全集钉形：ldexp（TIP 745）。
# 要点（对 tclsh9.1 实测）：
# - 第二实参是 Tcl_GetIntFromObj（64 位平台）：接受 [INT_MIN, UINT_MAX]，
#   超界报 `integer value too large to represent`；然后按 C int 截断
#   ——2147483648 与 2147483649 截成 -2147483648/-2147483647，结果都是
#   0.0（oracle 钉形），而 -2147483649 直接超界报错；
# - 溢出给 Inf、下溢给 0.0（ERANGE 接受），子正规结果（ldexp 1 -1074
#   = 5e-324）不报错；
# - 非整数（1.5/2.0/1e15）报 `expected integer but got`。
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t ld1 {tcl::mathfunc::ldexp 1.5 3}
t ld2 {tcl::mathfunc::ldexp 1 2147483647}
t ld3 {tcl::mathfunc::ldexp 1 2147483648}
t ld4 {tcl::mathfunc::ldexp 1 -2147483648}
t ld5 {tcl::mathfunc::ldexp 1 -2147483649}
t ld6 {tcl::mathfunc::ldexp 1 2147483649}
t ld7 {tcl::mathfunc::ldexp 1 4294967295}
t ld8 {tcl::mathfunc::ldexp 1 4294967296}
t ld9 {tcl::mathfunc::ldexp 1 9223372036854775807}
t ld10 {tcl::mathfunc::ldexp 1 -9223372036854775808}
t ld11 {tcl::mathfunc::ldexp 1 1024}
t ld12 {tcl::mathfunc::ldexp 1 -1023}
t ld13 {tcl::mathfunc::ldexp 1 -1074}
t ld14 {tcl::mathfunc::ldexp 1 -1075}
t ld15 {tcl::mathfunc::ldexp 4 -2}
t ld16 {tcl::mathfunc::ldexp 1 1.5}
t ld17 {tcl::mathfunc::ldexp 0.75 1}
t ld18 {tcl::mathfunc::ldexp 1 2.0}
t ld19 {tcl::mathfunc::ldexp 1 1e15}
t ld20 {tcl::mathfunc::ldexp 1.5 123456789012345678901234567890}
t ld21 {tcl::mathfunc::ldexp}
t ld22 {tcl::mathfunc::ldexp a b c}
t ld23 {tcl::mathfunc::ldexp 1 x}
t ld24 {tcl::mathfunc::ldexp 1 0x40000000}
t ld25 {tcl::mathfunc::ldexp 1 0b11}
t ld26 {expr {ldexp(0.75,1)}}
