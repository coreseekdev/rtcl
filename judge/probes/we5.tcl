# 空转（T11b 普查 2026-11-26）：两边死在行7：`namespace eval nn { set r [boomq] }` 没 catch，行10 的完成标记 L2-done 从未打印（错误报告本身有打印，是 sweep 既有 12 处 DIFF 之一）
proc boomq {} {error "boomq-msg"}
proc p {} {
    set q [boomq]
}
catch {p}
puts "L1<<$::errorInfo>>"
namespace eval nn {
    set r [boomq]
}
puts "L2-done"
