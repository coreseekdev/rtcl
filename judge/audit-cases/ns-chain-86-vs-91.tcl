namespace eval n {}
set g 1
set r [namespace eval n { set g 2 }]
puts "S1 write: result=$r global=$g ns=[catch {set n::g} m]$m"
namespace eval n { variable v 1 }
set v 9
set r [namespace eval n { append v x }]
puts "S2 declared: result=$r n::v=[set n::v] global=$v"
namespace eval n { unset v }
set r [namespace eval n { set v 7 }]
puts "S3 after-unset: result=$r n::v=[catch {set n::v} m]$m global=$v"
namespace eval n { variable w 1 }
namespace eval n { unset w }
set r [namespace eval n { set w 7 }]
puts "S4 same-body-after-unset: result=$r n::w=[catch {set n::w} m]$m"
proc P1 {} { set x secret; namespace eval n { set x 1; set y $x }; return [list $x [catch {set ::n::y} m] $m] }
puts "S5 from-proc: [P1]"
proc P2 {} { namespace eval n { set k2 7 } }
P2
puts "S6 from-proc-write: [info exists n::k2]"
set h 5
puts "S7 read-global: [catch {namespace eval n { set h }} m] $m"
array set arr {1 x}
puts "S8 array: [catch {namespace eval n { array get arr }} m] $m"
trace add variable tg write {apply {{a b c} { global log; lappend log [list $a $b $c] }}}
set log {}
namespace eval n { set tg 2 }
puts "S9 traced: global=[catch {set ::tg} m]$m ns=[catch {set n::tg} m2]$m2 log=$log"
puts "S10 [info patchlevel]"
namespace eval n { variable vg 1 }
set r [namespace eval n { list [set vg] [catch {set h} m] $m }]
puts "S11 declared-and-global: $r"
namespace eval n2 {}
set r [namespace eval n2 { set k 1; array set ka {1 x}; lappend k 2; incr c; dict set d a 1 }]
puts "S12 create-in-body: [catch {set n2::k} m]$m [catch {array get n2::ka} m]$m [catch {set n2::c} m]$m [catch {set n2::d} m]$m"
set r [namespace eval n2 { trace add variable tt write {apply {{a b c} {}}}; set tt 5 }]
puts "S13 traced-in-body: [catch {set n2::tt} m]$m global=[info exists ::tt]"
trace add variable gt write {apply {{a b c} { global log; lappend log [list $a $b $c] }}}
set log {}
set r [catch {namespace eval n2 { set gt 5 }} m]
puts "S14 global-trace-from-body: $r $m global=[catch {set ::gt} m2]$m2 ns=[catch {set n2::gt} m3]$m3 log=$log"
proc Q1 {} { global gx; namespace eval n2 { append gx x } }
set r [catch Q1 m]
puts "S15 global-link-from-proc-then-body: $r $m global=[catch {set ::gx} m]$m"
proc Q2 {} { variable n2::q; append q a; namespace eval n2 { append q b } }
set r [catch Q2 m]
puts "S16 variable-link-then-body: $r $m n2::q=[catch {set n2::q} m]$m"
namespace eval n2 { upvar #0 h hh; set hh 11 }
puts "S17 upvar0-in-body: h=[catch {set ::h} m]$m"
