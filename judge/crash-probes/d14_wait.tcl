# crash-probe: d14 wait
# gate: skip-tree
# 9.1 注意：tclsh 9.1 的 `wait` 命令已移除/重做（oracle 下脚本本体报错），
# 期望值只能来自 8.6.17 钉形。`wait` 的去留是迁移裁定项（TODO 对标迁移 T4
# 移除项）；裁定前本探针只在 rtcl 上防回归。
source [file join [file dirname [info script]] _chk.tcl]
# `wait` used to ask a fresh `sh` to wait for the pid — not the parent —
# and reported 127 for every child. The status now comes from the handle:
# {pid} CHILDSTATUS {status} once gone, {pid} NONE 0 under -nohang.
set p [exec sh -c {exit 0} &]
set e [wait $p]
chk shape [lindex $e 1] CHILDSTATUS
chk status [lindex $e 2] 0
set p [exec sh -c {exit 3} &]
set e [wait $p]
chk nonzero [lindex $e 2] 3
# Still running: -nohang reports it and leaves it alone.
set p [exec sleep 2 &]
set e [wait -nohang $p]
chk nohang $e "$p NONE 0"
# Reaped before `wait` asked (a later exec reaps finished children).
set p [exec sh -c {exit 5} &]
after 200
catch {exec true}
set e [wait $p]
chk reaped [lindex $e 2] 5
