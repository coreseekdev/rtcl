if {![info exists ::errorCode]} {set ::errorCode NONE}
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r|EC=$::errorCode" }
t close-w-on-wo {set f [open /tmp/rtcl_probe.txt w]; set r [catch {chan close $f write} m]; list $r $m [chan names]}
t close-w-then-close {set f [open /tmp/rtcl_probe.txt w]; chan close $f write; set r [catch {close $f} m]; list $r $m}
t close-w-then-puts {set f [open /tmp/rtcl_probe.txt w]; chan close $f write; set r [catch {puts $f x} m]; list $r $m $::errorCode}
t close-r-on-ro {set f [open /tmp/rtcl_probe.txt w]; set r [catch {chan close $f read} m]; list $r $m $::errorCode}
t close-rw-write {set f [open /tmp/rtcl_probe.txt w+]; chan close $f write; set r [catch {puts $f x} m]; set g [catch {seek $f 0} m2]; list $r $m $g $m2}
t close-rw-write-gets {set f [open /tmp/rtcl_probe.txt w+]; puts $f hello; chan close $f write; seek $f 0; set g [gets $f]; close $f; set g}
t copy-cmd-order {set g [open /tmp/rtcl_probe.txt w]; set r [catch {chan copy -size 2 stdin $g} m]; close $g; list $r $m}
t copy-after {set g [open /tmp/rtcl_probe.txt w]; set r [chan copy stdin $g -command cb2]; close $g; list $r}
proc cb2 {n} { set ::cbres $n }
t copy-after-cb {after 10; set ::cbres}
t copy4-msg {chan copy stdout stdin -size 1 x}
