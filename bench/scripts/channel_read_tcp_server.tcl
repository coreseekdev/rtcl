# P1 基准服务端：监听 127.0.0.1 临时端口，accept 后写 N 字节（默认
# 3MiB）即关。端口写进 T6_PORT_FILE 供客户端读取。用 tclsh9.1 跑
# （写侧不是测量对象，只要求灌满管道）。
# 用法：T6_PORT_FILE=... tclsh9.1 bench/scripts/channel_read_tcp_server.tcl [bytes]
set bytes 3145728
if {[llength $argv] > 0} { set bytes [lindex $argv 0] }
proc accept {chan addr p} {
    global bytes done
    fconfigure $chan -translation binary -buffering none
    puts -nonewline $chan [string repeat "x" $bytes]
    close $chan
    set done 1
}
set srv [socket -server accept 0]
set port [lindex [fconfigure $srv -sockname] 2]
set fh [open $::env(T6_PORT_FILE) w]
puts $fh $port
close $fh
vwait done
exit 0
