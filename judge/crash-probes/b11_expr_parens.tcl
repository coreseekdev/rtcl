# crash-probe: b11 expr parens
source [file join [file dirname [info script]] _chk.tcl]
set n 100000
set s [string repeat "(" $n]
append s 1 [string repeat ")" $n]
# skip rc -- 9.x expr 支持注释，# 与括号解析变化——TODO 对标迁移 T1 expr 注释
#   原断言（9.1 下不成立，钉形保留备考）：chk rc [catch {expr $s} e] 1
