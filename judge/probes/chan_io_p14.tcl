if {![info exists ::errorCode]} {set ::errorCode NONE}
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r|EC=$::errorCode" }
t ev-readable {chan event stdin readable {}}
t ev-readable-noscript {chan event stdin readable}
t ev-bad {chan event stdin bogus {}}
t ev-r {chan event stdin r {}}
t copy4 {chan copy stdout stdin -size 1 x}
t copy5 {chan copy stdout stdin -size 1 -command {}}
t copy-cmd {set g [open /tmp/rtcl_probe3.txt w]; chan copy stdout $g -command cb; close $g}
proc cb {n} { puts "callback:$n" }
