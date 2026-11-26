# tcl::process（9.1 子进程管理 ensemble）——空态面钉形（exec 子进程
# 随调用即收即清，常态 list/status 为空）；autopurge 默认 1、可置；
# 未知/歧义子命令的清单形。
proc ec {e} {expr {[dict exists $e -errorcode] ? [dict get $e -errorcode] : {}}}
proc try {script} {set c [catch {uplevel 1 $script} m e]; return "$c|$m|[ec $e]"}
puts [try {tcl::process}]
puts "auto=[tcl::process autopurge]"
puts "auto2=[tcl::process autopurge false][tcl::process autopurge]"
puts "auto3=[tcl::process autopurge true][tcl::process autopurge]"
puts "list=[tcl::process list]"
puts "status=[tcl::process status]"
puts "size=[llength [tcl::process list]] [dict size [tcl::process status]]"
puts [try {tcl::process gorp}]
puts [try {tcl::process aut}]
puts DONE
