# 空转（T11b 普查 2026-11-26）：两边死在行3：proc 体 `return $x` 裸名读不到全局（9.1 语义，can't read "x"），行4 未跑
set x 40
proc p {} { return $x }
puts "p=[p]"
puts "direct=[apply {{} {return $x}}]"
