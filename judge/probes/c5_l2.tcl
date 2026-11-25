# 空转（T11b 普查 2026-11-26）：两边死在行5：9.1 已无 `info line` 子命令（unknown subcommand "line"），行6–13 未跑
proc a1 {} {
    if {1} { nosuch }
}
catch {a1}
puts "arm: [info line nosuch] / err=[lindex [split [set ::errorInfo] \n] end-1]"
catch {a1} m
puts $::errorInfo
puts ===
proc a2 {} {
    if {0} { } else { nosuch }
}
catch {a2} m
puts $::errorInfo
