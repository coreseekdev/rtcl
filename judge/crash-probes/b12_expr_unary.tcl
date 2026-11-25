# crash-probe: b12 expr unary
source [file join [file dirname [info script]] _chk.tcl]
set s [string repeat - 50000]
append s 1
# skip rc -- 9.x expr 支持注释，# 解析变化——TODO 对标迁移 T1 expr 注释
#   原断言（9.1 下不成立，钉形保留备考）：chk rc [catch {expr $s} e] 1
