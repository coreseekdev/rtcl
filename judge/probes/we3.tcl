# 空转（T11b 普查 2026-11-26）：两边死在行2：`puts "A:[boomq]"` 没 catch 直接死，A: 标记从未打印
proc boomq {} {error "boomq-msg"}
puts "A:[boomq]"
