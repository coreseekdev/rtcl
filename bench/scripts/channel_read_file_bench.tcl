# P9 基准客户端：整读 32MB 文本文件（utf-8/auto 默认通道配置）。
# 引擎无关（rtcl -f / tclsh9.1 皆可）。数据文件经 T6_FILE 传入；
# 输出一行微秒数（read 的墙钟，不含 open/close 与解释器启动）。
# 用法：T6_FILE=... <engine> -f bench/scripts/channel_read_file_bench.tcl
set path $::env(T6_FILE)
set f [open $path r]
set t0 [clock microseconds]
set d [read $f]
set t1 [clock microseconds]
close $f
set size [file size $path]
if {[string length $d] != $size} {
    puts "BAD len=[string length $d] size=$size"
    exit 1
}
puts [expr {$t1 - $t0}]
