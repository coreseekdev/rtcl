# mf_mathop91.tcl — 9.0/9.1 新增 mathop 比较符钉形：lt/le/gt/ge
# （TIP 461，9.0 入 ::tcl::mathop）。要点（对 tclsh9.1 实测）：
# - 纯 UNICODE 字符串比较链（不做数值化）：lt 2 10 是 0（"2">"10"），
#   lt -inf -1 是 0（"i">'1'），lt 0x10 0o10 是 0（'x'>'o'）；
# - 少于两个实参恒真（0/1 个实参都是 1）；链式 AND 短路；
# - mathop-27..30.1..9 的语料形全覆盖 + 幾个边界（空串、Z/a 码点序）。
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t lt1 {::tcl::mathop::lt}
t lt2 {::tcl::mathop::lt a}
t lt3 {::tcl::mathop::lt a b}
t lt4 {::tcl::mathop::lt b a}
t lt5 {::tcl::mathop::lt a a}
t lt6 {::tcl::mathop::lt a b c}
t lt7 {::tcl::mathop::lt b a c}
t lt8 {::tcl::mathop::lt a c b}
t lt9 {::tcl::mathop::lt 012 0x0}
t lt10 {::tcl::mathop::lt 2 10}
t lt11 {::tcl::mathop::lt 10 2}
t lt12 {::tcl::mathop::lt abc abd}
t lt13 {::tcl::mathop::lt -inf -1}
t lt14 {::tcl::mathop::lt -2 -inf}
t lt15 {::tcl::mathop::lt 0x10 0o10}
t lt16 {::tcl::mathop::lt 1_2 3}
t lt17 {::tcl::mathop::lt 1.5 10}
t lt18 {::tcl::mathop::lt xX x}
t le1 {::tcl::mathop::le}
t le2 {::tcl::mathop::le a}
t le3 {::tcl::mathop::le a b}
t le4 {::tcl::mathop::le b a}
t le5 {::tcl::mathop::le a a}
t le6 {::tcl::mathop::le a b c}
t le7 {::tcl::mathop::le b a c}
t le8 {::tcl::mathop::le a c b}
t le9 {::tcl::mathop::le 012 0x0}
t le10 {::tcl::mathop::le a a b}
t le11 {::tcl::mathop::le {} a}
t le12 {::tcl::mathop::le 2 10 20}
t gt1 {::tcl::mathop::gt}
t gt2 {::tcl::mathop::gt a}
t gt3 {::tcl::mathop::gt a b}
t gt4 {::tcl::mathop::gt b a}
t gt5 {::tcl::mathop::gt a a}
t gt6 {::tcl::mathop::gt c b a}
t gt7 {::tcl::mathop::gt b a c}
t gt8 {::tcl::mathop::gt a c b}
t gt9 {::tcl::mathop::gt 0x0 012}
t gt10 {::tcl::mathop::gt 9 10}
t gt11 {::tcl::mathop::gt Z a}
t gt12 {::tcl::mathop::gt 10 2}
t ge1 {::tcl::mathop::ge}
t ge2 {::tcl::mathop::ge a}
t ge3 {::tcl::mathop::ge a b}
t ge4 {::tcl::mathop::ge b a}
t ge5 {::tcl::mathop::ge a a}
t ge6 {::tcl::mathop::ge c b a}
t ge7 {::tcl::mathop::ge b a c}
t ge8 {::tcl::mathop::ge a c b}
t ge9 {::tcl::mathop::ge 0x0 012}
t ge10 {::tcl::mathop::ge a a b}
t ge11 {::tcl::mathop::ge 20 10 2}
t ge12 {::tcl::mathop::ge b {}}
t chain1 {::tcl::mathop::lt a b c d e}
t chain2 {::tcl::mathop::lt e d c b a}
t chain3 {::tcl::mathop::le a a a a}
t count1 {llength [info commands ::tcl::mathop::*]}
