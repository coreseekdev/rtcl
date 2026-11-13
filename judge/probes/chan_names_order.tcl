# chan names = tclsh 8.6.17 channel-table hash order.  The table keys are
# fd-based (file0/1/2 for the std trio — tclUnixChan.c names channels
# "file%d"), stdin/stdout/stderr are display-layer substitutions in
# MatchChannels; iteration replays the Tcl_HashTable bucket order.
set d /tmp
proc t {label script} {
    if {![info exists ::errorCode]} {set ::errorCode NONE}
    set c [catch {uplevel 1 $script} r]
    puts "$label|$c|$r"
}
t std-only {set r [chan names]}
t one-open {set f [open $d/rtcl_names.txt w]; set r [list [chan names] [chan names file*] [chan names file3] [chan names file0] [chan names nope*]]; close $f; set r}
t three-open {set a [open $d/rtcl_n1.txt w]; set b [open $d/rtcl_n2.txt w]; set c2 [open $d/rtcl_n3.txt w]; set r [chan names]; close $a; close $b; close $c2; set r}
t mid-closed {set a [open $d/rtcl_n1.txt w]; set b [open $d/rtcl_n2.txt w]; close $b; set r [chan names]; close $a; set r}
t churn {for {set i 0} {$i < 3} {incr i} {set h [open $d/rtcl_ch.txt w]; set hh $h; close $h}; set hh}
