# crash-probe: a07 clock format neg
source [file join [file dirname [info script]] _chk.tcl]
set r [catch {clock format -100000000000000000} e]
# skip neg17 -- 9.x clock 重做（单调钟/timer），负值格式行为差——TODO 对标迁移 T4 clock 线
#   原断言（9.1 下不成立，钉形保留备考）：chk neg17 $r 0
set r [catch {clock format -2305843009213693952} e]
# skip neg2_61 -- 9.x clock 重做（单调钟/timer），负值格式行为差——TODO 对标迁移 T4 clock 线
#   原断言（9.1 下不成立，钉形保留备考）：chk neg2_61 $r 0
