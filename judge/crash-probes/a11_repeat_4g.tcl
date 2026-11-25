# crash-probe: a11 repeat 4g
source [file join [file dirname [info script]] _chk.tcl]
# skip lr -- 9.x 64-bit 容量（TIP 626），lrepeat 4G 合法——TODO 对标迁移 T2
#   原断言（9.1 下不成立，钉形保留备考）：chk lr [catch {lrepeat 4294967296 x} e] 1
# skip sr -- 9.x 64-bit 容量（TIP 626），lrepeat 4G 合法——TODO 对标迁移 T2
#   原断言（9.1 下不成立，钉形保留备考）：chk sr [catch {string repeat x 4294967296} e] 1
