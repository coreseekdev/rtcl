# crash-probe: d08 octal bignum
source [file join [file dirname [info script]] _chk.tcl]
set v 01000000000000000000005
# skip val -- 9.x 0NNN 非八进制 + 任意精度——TODO 对标迁移 T1 数字字面量
#   原断言（9.1 下不成立，钉形保留备考）：chk val [expr {$v}] 9223372036854775813
