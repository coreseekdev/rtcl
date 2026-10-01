if {![info exists ::errorCode]} {set ::errorCode NONE}
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r|EC=$::errorCode" }
t puts-on-ro {set f [open /tmp/rtcl_probe2.txt r]; set r [catch {puts $f hi} m]; close $f; list $r $m $::errorCode}
t gets-on-wo {set f [open /tmp/rtcl_probe2.txt w]; set r [catch {gets $f} m]; close $f; list $r $m $::errorCode}
t read-on-wo {set f [open /tmp/rtcl_probe2.txt w]; set r [catch {read $f} m]; close $f; list $r $m $::errorCode}
t copy-1arg {chan copy foo}
t copy-junk2 {set src [open /tmp/rtcl_probe2.txt]; set g [open /tmp/rtcl_probe.txt w]; set r [catch {chan copy $src $g extra junk} m]; close $src; close $g; list $r $m}
t copy-prefix-opt {set src [open /tmp/rtcl_probe2.txt]; set g [open /tmp/rtcl_probe.txt w]; set r [catch {chan copy $src $g -s 3} m]; close $src; close $g; list $r $m}
t copy-src-missing {set g [open /tmp/rtcl_probe.txt w]; set r [catch {chan copy nosuch $g} m]; close $g; list $r $m $::errorCode}
t copy-dst-missing {set src [open /tmp/rtcl_probe2.txt]; set r [catch {chan copy $src nosuch} m]; close $src; list $r $m $::errorCode}
t copy-cmd-nonexistent {set src [open /tmp/rtcl_probe2.txt]; set g [open /tmp/rtcl_probe.txt w]; set r [catch {chan copy $src $g -command nosuchproc} m]; close $src; close $g; list $r $m $::errorCode}
