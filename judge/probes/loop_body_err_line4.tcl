# for 体帧行号（fuzz f000272 的行号面收口钉形；该样本的 ARITH 首帧
# 关键词残差另账——见 DIVERGENCES 2026-11-17 节，双引擎共有）。行 =
# 词括号属主命令体相对行 2。尾段未捕获钉形。
catch {for {set i 0} {$i < 1} {incr i} {set s [expr {~($x)}]}} m o
puts "A:EL=[dict get $o -errorline]"
for {set m 0} {$m < 7} {incr m 2} {
  set n 0x10
  set s [expr {~($x)}]
}
