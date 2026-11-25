# 空转（T11b 普查 2026-11-26）：两边死在行8：`switch a {a - - b …}` 7 元素不成对（extra switch pattern with no body），行9–11 的 errorInfo 钉形段未跑
puts [switch abc a* {subst glob} abc {subst exact}]
puts [catch {switch -glob x default {subst 1}} r]; puts $r
puts [catch {switch -nocase x DEFAULT {subst 1}} r]; puts $r
puts [switch -nocase default DEFAULT {subst lit} fallback {subst 2}]
puts [catch {switch x a} r]; puts $r
puts [catch {switch -- x} r]; puts $r
puts [catch {switch x a {b}} r]; puts $r
puts [switch a {a - - b {subst deep} c {subst 2}}]
puts [switch b {a - - b {subst deep} c {subst 2}}]
set e [catch {switch a a {error E1}} m]; puts "$e $m"
puts $::errorInfo
