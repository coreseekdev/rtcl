# S21 probe: lrepeat / string repeat 的大小守卫按 9.1 钉形。
#
# tclsh 9.1 的容量语义（TIP 626，64 位 Tcl_Size）：
#   - lrepeat 的 count 不再有 8.6 的 C int 上限；负数报 NEGARG；
#     objc*count 超过 LIST_MAX（本机 9.1.0 x86-64 = 1152921504606846970，
#     (TCL_SIZE_MAX - offsetof(ListStore,slots))/8）报
#     `max length (N) of a Tcl list exceeded` + TCL MEMORY；
#     更大的 count 走惰性 lrepeatType 表示，rtcl 急切物化、对超预算
#     的按同样形状拒绝（惰性成功带 rtcl 拒绝的那一段没有逐字节可比
#     的 oracle，不进本探针）。
#   - string repeat：count 超 i64 报 ARITH IOVERFLOW；不是数字报
#     expected integer + TCL VALUE NUMBER；count >= TCL_SIZE_MAX 报
#     `max size for a Tcl value (9223372036854775807 bytes) exceeded`；
#     alloc 失败报 `string size overflow: unable to alloc N bytes`——
#     rtcl 的物化预算（2 GiB）之上直接用后者的形状，天文数字的 count
#     在 tclsh 上也必然 alloc 失败，两边逐字节一致。
# 期望值全部按 tclsh 9.1.0 实测。
set ::errorCode NONE

proc T {script} {
    set c [catch {uplevel 1 $script} r]
    return [list $c [string range $r 0 80] $::errorCode]
}

# ── lrepeat ──────────────────────────────────────────────────────────
puts "lr-neg=[T {llength [lrepeat -1 x]}]"
puts "lr-neg64=[T {llength [lrepeat -2147483649 x]}]"
puts "lr-nan=[T {llength [lrepeat notanumber x]}]"
puts "lr-float=[T {llength [lrepeat 1.5 x]}]"
puts "lr-bignum=[T {llength [lrepeat [expr {10**19}] x]}]"
puts "lr-wmax=[T {llength [lrepeat 9223372036854775807 x]}]"
puts "lr-0=[T {llength [lrepeat 0 a b c]}]"
puts "lr-small=[T {llength [lrepeat 3 a b c]}]"

# ── string repeat ────────────────────────────────────────────────────
puts "sr-nan=[T {string length [string repeat x notanumber]}]"
puts "sr-chain=[T {string length [string repeat x 1+2]}]"
puts "sr-bignum=[T {string length [string repeat x [expr {10**19}]]}]"
puts "sr-neg=[T {string length [string repeat x -1]}]"
puts "sr-empty=[T {string length [string repeat {} 4294967296]}]"
puts "sr-wmax=[T {string length [string repeat x 9223372036854775807]}]"
puts "sr-wmax-1=[T {string length [string repeat x 9223372036854775806]}]"
puts "sr-unicode-wmax=[T {string length [string repeat é 9223372036854775807]}]"
puts "sr-ok=[string repeat ab 3]"
puts done
