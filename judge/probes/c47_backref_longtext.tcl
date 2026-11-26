# C47（9.1 对齐批 B1，2026-11-26）：长文本上的回溯 regexp 给出结果而非
# REGEXP REG_ESPACE。oracle（tclsh 9.1.0）实测：步数预算对
# `(x+)y\1`（1030/2000/3300 个 x，无 y）等线性-平方扫描族给出 **0**
# （找不到匹配、完成扫描）；带 y 的匹配形给 1。原 rtcl 2M 步预算在
# n≈1000 就报 regular expression match is too complex。
# 预算仍是防挂死护栏：真指数级 pattern（bail 墙钟 ~2s）照报。
set s [string repeat x 1030]
puts "A: [regexp {(x+)y\1} $s]"
set s2 "[string repeat x 1030]y[string repeat x 1030]"
puts "B: [regexp {(x+)y\1} $s2]"
set s3 [string repeat a 2000]
puts "C: [regexp {(a+)\1} $s3]"
set s4 "[string repeat a 1500]b"
puts "D: [regexp {(a+)b} $s4]"
set s5 [string repeat x 5000]
puts "E: [regexp {(x+x+)+y} $s5]"
set s6 "[string repeat x 100]y[string repeat x 100]"
puts "F: [regexp {(x+)y\1} $s6]"
puts "G: [regexp {(x*)y\1*} [string repeat x 2000]y]"
foreach n {1000 2000 3300} {
    set s [string repeat x $n]
    set c [catch {regexp {(x+)y\1} $s} m]
    puts "n=$n c=$c m=$m"
}
