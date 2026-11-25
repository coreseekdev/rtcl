# 空转（T11b 普查 2026-11-26）：两边死在行7解析：`t {$foo({)}` 大括号失衡（missing close-brace），行8–13 未跑
proc t {e} {
    if {[catch {expr $e} m]} { puts "ERR <<$e>>: $m" } else { puts "OK  <<$e>>: $m" }
}
t {$foo("x)}
t {$foo(x"y)}
t {$foo([x"y])}
t {$foo({)}
t {$foo("x")}
t {$foo([x])}
t {$foo($x)}
t {$foo(a{b}c)}
t {$foo("x"y)}
t {$foo([abcdefghijklmnopqrstuvwxyz"")]}
