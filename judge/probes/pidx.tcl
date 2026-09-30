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
