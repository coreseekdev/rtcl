# foreach 体帧行号（fuzz f000246 收口钉形）：("foreach" body line N) 同
# 族——行基 = 词括号 defer 行（属主命令绝对行）- 体首行 + 1。catch 体
# 无体帧（非文件语境）。尾段未捕获钉形。
catch {foreach i {1 2} {set y [expr {~($x)}]}} m o
puts "A:EL=[dict get $o -errorline]"
foreach b {4 -3} n {3 5 -4} {
  set x [expr {$a * $b}]
  continue
}
