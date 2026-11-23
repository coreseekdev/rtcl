# crash-probe: a03 bareword cjk
source [file join [file dirname [info script]] _chk.tcl]
set mode 有効なモードです
chk bareword [catch {if $mode {puts yes}} e] 1
chk bareword2 [catch {expr 中文中文中文中文中文中文中文中文文} e] 1
