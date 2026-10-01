if {![info exists ::errorCode]} {set ::errorCode NONE}
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r|EC=$::errorCode" }
t close-dir-bad {set f [open /tmp/rtcl_probe.txt w]; set r [catch {chan close $f bogus} m]; list $r $m $::errorCode}
t close-dir-w {set f [open /tmp/rtcl_probe.txt w]; set r [catch {chan close $f write} m]; list $r $m}
t close-dir-input {set f [open /tmp/rtcl_probe.txt w]; set r [catch {chan close $f input} m]; list $r $m}
t postevent-rw {chan postevent stdin readable}
t postevent-bad {set r [catch {chan postevent stdin bogus} m]; list $r $m $::errorCode}
t names-sorted {chan names}
