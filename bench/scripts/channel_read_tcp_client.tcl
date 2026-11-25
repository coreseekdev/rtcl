# P1 基准客户端：连 127.0.0.1，`read $s` 到 EOF（默认 utf-8/auto 配置），
# 输出一行微秒数。端口从 T6_PORT_FILE 读（服务端是 socket -server，
# rtcl 不支持，故恒由 tclsh9.1 扮演；测的是本进程的读）。
# 用法：T6_PORT_FILE=... <engine> -f bench/scripts/channel_read_tcp_client.tcl
set fh [open $::env(T6_PORT_FILE) r]
set port [string trim [read $fh]]
close $fh
set s [socket 127.0.0.1 $port]
set t0 [clock microseconds]
set d [read $s]
set t1 [clock microseconds]
close $s
if {[string length $d] != 3145728} {
    puts "BAD len=[string length $d]"
    exit 1
}
puts [expr {$t1 - $t0}]
