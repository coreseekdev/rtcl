set v global-v
set a(g) arr-g
namespace eval n {}
namespace eval n {
    set ::r1 [set v]
    set v ns-v
    set ::r2 [set v]
    set ::r3 [set ::v]
    set ::r4 [set a(g)]
    set a(g) arr-g-mod
    set ::r5 [set a(g)]
    set ::r6 [info exists a(n1)]
}
puts "R1=$r1 R2=$r2 R3=$r3 R4=$r4 R5=$r5 R6=$r6"
puts "GV=[set ::v] NAV=[info exists ::n::v] AG=[set ::a(g)] NA=[info exists ::n::a]"
namespace eval n { unset v; set ::r7 [catch {set v} m7]; set ::r8 [set ::v] }
puts "R7=$r7 M7=$m7 R8=$r8"
namespace eval n { unset a; set ::r9 [catch {set a(g)} m9]; set ::m9 $m9 }
puts "R9=$r9 M9=$m9 AG2=[catch {set ::a(g)} m10]; $m10"
namespace eval n { set ::r11 [set errorCode]; set ::r12 [info vars] }
puts "R11=$r11 VARS=$r12"
