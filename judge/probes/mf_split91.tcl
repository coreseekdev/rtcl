# mf_split91.tcl — 9.1 新命令钉形：divmod/frexp/modf/remquo（TIP 745）。
# 要点（对 tclsh9.1 实测）：
# - divmod 宽整路径保留上游 quirk：商为负时的判断是 (q*y) != y
#   （8.6 是 != x），所以整除的负商也再减一——divmod -6 1 是 {-7 1}、
#   divmod -100 10 是 {-11 10}，与 / 和 % 不一致；bignum 路径则是
#   真正的 floor 除法；MIN/-1 上游 SIGFPE，rtcl wrapping 兜住不崩；
# - 非整数数字报 `dividend/divisor must be an integer`；除零报
#   `divide by zero`；非数字报 `expected number but got`；
# - frexp/modf/remquo 走 GetDoubleFromObj（"nan" 串拒绝）；-0.0 的
#   符号在两个分解里都保持；frexp(inf) 是 {Inf 0}；remquo 的商带
#   x/y 符号（12.5/-1.5 → -8）。
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t dm1 {divmod 15 4}
t dm2 {divmod 155 -4}
t dm3 {divmod -6 3}
t dm4 {divmod -5 3}
t dm5 {divmod 6 -3}
t dm6 {divmod 5 -3}
t dm7 {divmod -1 -4}
t dm8 {divmod 1 -4}
t dm9 {divmod -6 1}
t dm10 {divmod -1 1}
t dm11 {divmod -100 10}
t dm12 {divmod -20 10}
t dm13 {divmod 100 -10}
t dm14 {divmod -7 2}
t dm15 {divmod 7 -2}
t dm16 {divmod -1 4}
t dm17 {divmod 0 -4}
t dm18 {divmod -9223372036854775807 -1}
t dm19 {divmod 0x10 3}
t dm20 {divmod 0x10 5}
t dm21 {divmod 1.5 0.5}
t dm22 {divmod 15.0 3}
t dm23 {divmod a b}
t dm24 {divmod 15 a}
t dm25 {divmod 12 4.0}
t dm26 {divmod 15 0}
t dm27 {divmod 15151515151515151515151515151515151515151515151515151515 0}
t dm28 {divmod 15151515151515151515151515151515151515151515151515151515 44444444444444444444444444444444444444444444444}
t dm29 {divmod 15151515151515151515151515151515151515151515151515151515 -4444444444444444444444444444444444444444444444}
t dm30 {divmod 9223372036854775807 9223372036854775807}
t dm31 {divmod -9223372036854775808 9223372036854775807}
t dm32 {divmod}
t dm33 {divmod a b c}
t fr1 {frexp 1.5}
t fr2 {frexp 0}
t fr3 {frexp -0.0}
t fr4 {frexp inf}
t fr5 {frexp -inf}
t fr6 {frexp 1e-320}
t fr7 {frexp 12345678901234567890123456789012345678901234567890}
t fr8 {frexp a}
t fr9 {frexp 5}
t fr10 {frexp 123.456}
t fr11 {frexp}
t fr12 {frexp a b}
t fr13 {frexp 5e-324}
t mo1 {modf 1.5}
t mo2 {modf 0}
t mo3 {modf -1.5}
t mo4 {modf inf}
t mo5 {modf -inf}
t mo6 {modf 1e-320}
t mo7 {modf 5}
t mo8 {modf a}
t mo9 {modf 123.456}
t mo10 {modf -0.0}
t mo11 {modf 5e-324}
t mo12 {modf}
t mo13 {modf a b}
t rq1 {remquo 5 2}
t rq2 {remquo -5 2}
t rq3 {remquo 5 -2}
t rq4 {remquo -5 -2}
t rq5 {remquo 1 inf}
t rq6 {remquo 12.3 3.89}
t rq7 {remquo 5 0.5}
t rq8 {remquo 0 1}
t rq9 {remquo 5 4.9}
t rq10 {remquo 1 3}
t rq11 {remquo 12.5 1.5}
t rq12 {llength [remquo 12.5 1.5]}
t rq13 {lindex [remquo 12.5 1.5] 0}
t rq14 {lindex [remquo 12.5 -1.5] 0}
t rq15 {lindex [remquo 12.5 -1.5] 1}
t rq16 {remquo}
t rq17 {remquo a b c}
t rq18 {remquo 1 a}
t rq19 {expr {fpclassify([lindex [remquo 1 inf] 0]) eq "nan"}}
