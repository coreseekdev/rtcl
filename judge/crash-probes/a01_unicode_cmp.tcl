# crash-probe: a01 unicode cmp
source [file join [file dirname [info script]] _chk.tcl]
set name 张三
if {$name < "x"} { set r lt } else { set r ge }
chk cmp $r ge
chk eq [expr {"日本語" == "x"}] 0
chk lt [expr {"héllo" < "x"}] 1
