proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r|EC=$::errorCode" }
t copy1 {chan copy stdout}
t copy4 {chan copy stdout stdin -size 1 x}
t copy-size-noint {catch {chan copy stdout stdin -size zz} m; set m}
t copy-dir {catch {chan copy stdin stdout} m; set m}
t copy-2read {set f [open /tmp/rtcl_probe.txt r]; set g [open /tmp/rtcl_probe2.txt w]; set r [catch {chan copy $f $g} m]; close $f; close $g; list $r $m}
