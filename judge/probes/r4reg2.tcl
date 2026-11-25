# 空转（T11b 普查 2026-11-26）：两边死在行4：apply lambda 体 `return $x` 裸名读不到全局（9.1 语义，can't read "x"），行5–6 未跑
set x 40
apply {{} { set y 5 }}
puts "apply-set-y=[catch {set y} m]; m=$m"
puts "g=[apply {{} {return $x}}]"
proc p {} { return $x }
puts "p=[p]"
