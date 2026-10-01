if {![info exists ::errorCode]} {set ::errorCode NONE}
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r|EC=$::errorCode" }
t close-nosuch-dir {set r [catch {chan close nosuch read} m]; list $r $m $::errorCode}
t close-nosuch-bogus {set r [catch {chan close nosuch bogus} m]; list $r $m $::errorCode}
t close-ro-read {set f [open /tmp/rtcl_probe2.txt r]; set r [catch {chan close $f read} m]; list $r $m [llength [chan names]]}
t close-rw-read-ec {set f [open /tmp/rtcl_probe.txt w+]; set r [catch {chan close $f read} m]; list $r [list $m] [string length $m] $::errorCode}
t close-stdin-read {set r [catch {chan close stdin read} m]; list $r $m $::errorCode}
t cfg-noargs {fconfigure}
t chan-cfg-noargs {chan configure}
t chan-cfg-odd {set f [open /tmp/rtcl_probe.txt w]; set r [catch {chan configure $f -blocking} m]; close $f; list $r $m}
t close-file-bogus-dir-ec {set f [open /tmp/rtcl_probe.txt w]; set r [catch {chan close $f b} m]; list $r $m $::errorCode}
t close-prefix-r {set f [open /tmp/rtcl_probe.txt w]; set r [catch {chan close $f w} m]; list $r $m}
