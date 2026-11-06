# 编译错误码/消息与错误优先级（oracle: tclsh 8.6.17）。
# 用法同其它 probes：两引擎输出 diff 应为空。
proc t {hex} {
    set p [binary decode hex $hex]
    set ::errorCode {}
    if {[catch {regexp $p x} m]} {
        puts "P<$p> => ERR <$m> / [lindex $::errorCode 1]"
    } else {
        puts "P<$p> => OK"
    }
}

# 类内坏范围 vs 后续 escape/EOF 的优先级。
t 5b7a2d615c
t 5b615c
t 5b7a2d615c71
t 5b7a2d615c78
t 5b7a2d615c31
t 5b7b2d6e5c
t 5b7b2d6e2b5c
t 5b7a2d612b5c
t 5b7a2d6120625c
t 5b7a2d61325c
t 5b242d3224297b2d6e2b5c
t 5b7a2d61245c
t 5b7a2d61295c28
t 5b7a2d612b5c71
t 5b7a2d615c20
t 5b7a2d615b5b3a666f6f3a5d5d
t 5b7b202d6e

# 常见 REG_* 错误码。
t 2a
t 612a2a
t 28
t 29
t 5b61
t 5b7a2d615d
t 5b5b3a666f6f3a5d5d
t 617b322c317d
t 617b3235367d
t 5c71
t 5c31
t 5b5c575d
t 5b5b3d61623d5d5d
t 5b5b2e61622e5d5d
t 283f7a29
t 283f3c3d612962
t 617b32

# 首个 ] 作为 range 起点 / 未闭合类尾随 - / escape 优先级。
t 5b5d2d615d
t 5b5d2d61
t 5b612d5d5d
t 5b5d2d3e5c5d
t 5b5d2d3e5c7b
t 5b5d2d295c5d
t 5b7a2d615c5d
t 5b5d2d3e5d
t 5b5d295c2d5d
t 5b5d2d615c
t 5b5d2d615c715d
t 5b612d622d
t 5b612d622d63
t 5b612d
t 5b61622d
t 5b612d622d5d
t 5b612d622d635d
