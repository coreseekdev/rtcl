# E1 io/string 热路径基准（E1 批验收用）：string index/range/first/last
# （parse_tcl_index 入口）、open/read 通道读、string repeat。
# 引擎无关；stdout 出 name|µs 行。用法：<engine> -f e1_iostr_bench.tcl
proc timed {reps body} {
    set t0 [clock microseconds]
    for {set i 0} {$i < $reps} {incr i} {
	uplevel 1 $body
    }
    return [expr {[clock microseconds] - $t0}]
}
set text [string repeat "abcdefgh" 4000]; ;# 32KB ascii
set n [string length $text]
puts "ix-hit|[timed 200000 {string index $text 12345}]"
puts "ix-end|[timed 200000 {string index $text end-7}]"
puts "rg|[timed 100000 {string range $text 100 1100}]"
puts "first|[timed 50000 {string first hgh $text 30000}]"
puts "last|[timed 50000 {string last hgh $text}]"
puts "len|[timed 500000 {string length $text}]"
# 通道读：写一个 8MB 文件再整读 ×3
set p [file join [pwd] __e1bench.dat]
set f [open $p w]
puts -nonewline $f [string repeat $text 256]
close $f
set f [open $p r]
puts "chan-read|[timed 40 {set d [read $f]; seek $f 0}]"
close $f
file delete $p
puts DONE
