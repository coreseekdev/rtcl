# uplevel 落 namespace eval 帧的 level 计数（tclsh：目标 varFrame 保持
# 存活——info level 计 ns eval 一层、info level 0 报 namespace eval 调用
# 词；uplevel 0 在 ns eval 体 = 当前帧。2026-11-14 errorInfo 帧组轮）。
proc q {} { uplevel 0 {info level} }
puts "A:[q]"
catch {namespace eval e2 {info level 0}} m
puts "B:$m"
puts "C:[namespace eval e3 {namespace eval e4 {info level}}]"
catch {uplevel 0 {info level}} m2
puts "D:$m2"
namespace eval e5 { puts "E:[uplevel 0 {info level}]" }
proc q2 {} { namespace eval e6 { info level } }
puts "F:[q2]"
namespace eval t { proc s {} { uplevel 1 {info level} } }
namespace eval t { puts "G:[s]" }
proc outer {} { namespace eval inner { uplevel 1 {info level} } }
puts "H:[outer]"
namespace eval u { proc s2 {} { uplevel 2 {info level} } }
namespace eval u { proc o2 {} { uplevel 1 {s2} } }
namespace eval u { puts "I:[o2]" }
puts "J:[namespace eval v {info level}]"
proc w {} { namespace eval v2 { info level } }
puts "K:[w]"
catch {t::s} m3
puts "L:[string range $m3 0 0]"
