# C 阶段 9.1 对齐（2026-11-26）：unset trace 回调的 name1 形。
# oracle（tclsh 9.1.0）实测：
# - `unset ta(k)` 直元素 unset：name1 = **unset 写下的全名**（ta(k)、
#   ::ta(k) 带限定都原样），name2 = 元素；
# - `unset tb2`（数组整体）与 `array unset td` 的扫描：name1 = 裸数组
#   名（tb2/td），name2 = 元素；
# - write/read trace 与标量 unset：name1 = 裸基名、标量 n2 = 空
#   （原 rtcl 行为不变——只有直元素 unset 的 name1 从 ta 改成 ta(k)）。
set ::c 0
proc tr {n1 n2 op} { incr ::c; puts "T$::c: n1=$n1 n2=$n2 op=$op" }
set ta(k) 1
trace add variable ta(k) unset tr
unset ta(k)
array set tb {k 1 j 2}
trace add variable tb(j) unset tr
unset tb(j)
array set tb2 {k 1 j 2}
trace add variable tb2(j) unset tr
unset tb2
set sa 5
trace add variable sa unset tr
unset sa
array set tc {k 1}
trace add variable tc(k) unset tr
unset ::tc(k)
array set td {k 1 j 2}
trace add variable td(j) unset tr
array unset td
array set te {k 1}
trace add variable te(k) unset tr
catch {unset te(nosuch)}
array set tf {k 1}
trace add variable tf(k) write tr
set tf(k) 2
trace add variable tf(k) read tr
set y $tf(k)
