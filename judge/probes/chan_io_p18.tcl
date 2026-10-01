if {![info exists ::errorCode]} {set ::errorCode NONE}
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r|EC=$::errorCode" }
set f [open /tmp/rtcl_probe2.txt w]; puts $f "ab\ncd"; close $f
t rw-close-w {set f [open /tmp/rtcl_probe.txt w+]; set r [catch {chan close $f write} m]; list $r [list $m] [string length $m] $::errorCode}
t rw-close-r {set f [open /tmp/rtcl_probe.txt w+]; set r [catch {chan close $f read} m]; list $r [list $m] [string length $m] $::errorCode}
t ro-close-w {set f [open /tmp/rtcl_probe.txt r]; set r [catch {chan close $f write} m]; list $r $m $::errorCode}
t close-w-twice {set f [open /tmp/rtcl_probe.txt w]; chan close $f write; set r [catch {chan close $f write} m]; list $r $m $::errorCode}
t tell-after-close {set f [open /tmp/rtcl_probe.txt w]; chan close $f write; set r [catch {tell $f} m]; list $r $m}
t copy-size-neg {set src [open /tmp/rtcl_probe2.txt]; set g [open /tmp/rtcl_probe.txt w]; set r [catch {chan copy $src $g -size -1} m]; close $src; close $g; list $r $m}
t copy-unknown-opt {set src [open /tmp/rtcl_probe2.txt]; set g [open /tmp/rtcl_probe.txt w]; set r [catch {chan copy $src $g -bogus 1} m]; close $src; close $g; list $r $m}
t copy-basic {set src [open /tmp/rtcl_probe2.txt]; set dst [open /tmp/rtcl_probe3.txt w]; set n [chan copy $src $dst]; close $src; close $dst; set v [open /tmp/rtcl_probe3.txt]; set d [read $v]; close $v; list $n $d}
t copy-size {set src [open /tmp/rtcl_probe2.txt]; set dst [open /tmp/rtcl_probe3.txt w]; set n [chan copy $src $dst -size 3]; close $src; close $dst; set v [open /tmp/rtcl_probe3.txt]; set d [read $v]; close $v; list $n $d}
