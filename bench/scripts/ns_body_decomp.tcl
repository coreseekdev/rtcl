# T4 分解：ns 体里各 op 类型单独计段（time 报 µs/迭代，段 5M 次）。
# 注：段内计数变量 i 走 incr i（ns 体裸名）——各行同含这份底噪，
# empty 段给出「裸 set + incr」底噪基线。
set n 5000000
namespace eval nb { variable v 5; array set arr {1 x} }
namespace eval nb {
    puts "empty=[time {set z 0} $::n]"
    puts "read=[time {set v} $::n]"
    puts "write=[time {set v 7} $::n]"
    puts "arrread=[time {set arr(1)} $::n]"
    puts "arrwrite=[time {set arr(1) 7} $::n]"
    puts "incr=[time {incr i} $::n]"
    puts "qread=[time {set ::n} $::n]"
}
puts DONE
