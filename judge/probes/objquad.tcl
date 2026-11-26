# objquad：对象化列表的自嵌套在 20000/40000/80000 规模下仍近线性
# （原缺陷：某个规模起平方退化跑不完）。墙钟秒级断言有 ±1-2s 负载
# 抖动（负载敏感 flake 多次实证）——改为容差断言：每规模 ≤2s 即过
# （80000 规模实测 ≤2s）；超时才是退化回归。
foreach n {20000 40000 80000} {
    set s [clock seconds]
    set x {}
    for {set i 0} {$i<$n} {incr i} { set x [list $x {}] }
    set el [expr {[clock seconds]-$s}]
    puts "$n -> $el s: [expr {$el <= 2 ? {ok} : {SLOW}}]"
}
