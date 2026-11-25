#!/usr/bin/env tclsh
# P6 实测：变量表哈希对同前缀/同长名字族的散布（interp/varmap.rs
# VarHasher）。三族名字各建 N 个变量，再整表按名读回 R 轮，报告每
# 次读的整段微秒。族：
#   seq7 : k000000..kN（7 字节、仅尾字节异——单字乘法哈希低位扩散
#          不动的经典退化族）
#   var  : k0..kN（变长名字，对照族）
#   mid  : kx00000y..（首尾定、中段六字节异的同长族）
# 用法：T7_VH_N=60000 T7_VH_R=5 tclsh rtcl 均可跑；stdout 每族一行
# “seq7 <us>”。

set N 60000
if {[info exists ::env(T7_VH_N)]} { set N $::env(T7_VH_N) }
set R 5
if {[info exists ::env(T7_VH_R)]} { set R $::env(T7_VH_R) }

proc build {kind n} {
    for {set i 0} {$i < $n} {incr i} {
        switch -- $kind {
            seq7 { set name [format "k%06d" $i] }
            var  { set name "k$i" }
            mid  { set name [format "kx%06dy" $i] }
        }
        upvar #0 $name v
        set v $i
    }
}

proc readloop {kind n r} {
    set s 0
    for {set j 0} {$j < $r} {incr j} {
        for {set i 0} {$i < $n} {incr i} {
            switch -- $kind {
                seq7 { set name [format "k%06d" $i] }
                var  { set name "k$i" }
                mid  { set name [format "kx%06dy" $i] }
            }
            upvar #0 $name v
            incr s $v
        }
    }
    return $s
}

foreach kind {seq7 var mid} {
    build $kind $N
    set t0 [clock microseconds]
    set s [readloop $kind $N $R]
    set us [expr {[clock microseconds] - $t0}]
    puts stderr "sink=$s"
    puts "$kind $us"
}
