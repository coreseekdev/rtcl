# return -code/-level 穿越循环体与边界的完成码矩阵（tclsh
# TclProcessReturn/TclUpdateReturnInfo 钉形，2026-11-14 errorInfo 帧组轮）：
# TCL_RETURN 不在循环边界解码、proc 边界降级、-level 0 物化原码。
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t lm-collect-all {lmap i {a b c} { return -level 0 $i }}
t lm-keeps-going {lmap i {a b c} { if {$i eq "b"} { return -level 0 $i }; set _ }}
t lm-empty {lmap i {a b c} { return -level 0 }}
t lm-code-return {lmap i {a b c} { return -code return $i }}
t lm-code-break {lmap i {a b c} { return -code break $i }}
t lm-code-continue {lmap i {a b c} { return -code continue $i }}
t lm-code-error {lmap i {a b c} { return -code error $i }}
t lm-return-in-proc {proc p {} { lmap i {a b c} { return $i } }; p}
t lm-code-return-level0 {proc p {} { lmap i {a b c} { return -code return -level 0 $i } }; p}
t fe-level0 {proc p {} { foreach i {a b c} { return -level 0 $i }; set done 1 }; p}
t fe-code-return {proc p {} { foreach i {a b c} { return -code return $i }; set done 1 }; p}
t fe-level0-keeps-going {proc p {} { foreach i {a b c} { if {$i=="b"} {return -level 0 $i} } ; list after $i }; p}
t for-level0 {proc p {} { for {set i 0} {$i<3} {incr i} { return -level 0 $i } ; list after $i }; p}
t fe-cr {proc p {} { foreach i {a b c} { return -code return $i }; set d 1 }; p}
t fe-ok {proc p {} { foreach i {a b c} { return $i }; set d 1 }; p}
t fe-err {proc p {} { foreach i {a b c} { return -code error E }; set d 1 }; p}
t fe-brk {proc p {} { foreach i {a b c} { return -code break }; set d 1 }; p}
t for-cr {proc p {} { for {set i 0} {$i<1} {incr i} { return -code return R }; set d 1 }; p}
t wh-cr {proc p {} { while {1} { return -code return W } }; p}
t fe-cr-tl { foreach i {a b c} { return -code return $i } }
puts "ret:[catch {return -level 0 -code return rr} a] a=$a"
puts "cont:[catch {for {set i 0} {$i < 3} {incr i} {if {$i == 1} {return -level 0 -code continue}}} b] b=$b i=$i"
set s {}
set r [catch {foreach x {1 2 3} {if {$x == 2} {return -level 0 -code continue}; lappend s $x}} b2]
puts "fc: r=$r s=$s b=$b2"
puts "int7:[catch {return -level 0 -code 7 zz} c] c=$c"
set d {}
set r2 [catch {while {1} {incr d(0); return -level 0 -code error eek}} d2]
puts "werr: r=$r2 d=[array get d] m=$d2"
