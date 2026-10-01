if {![info exists ::errorCode]} {set ::errorCode NONE}
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r|EC=$::errorCode" }
set f [open /tmp/rtcl_probe.txt w]
t set-then-extra {fconfigure $f -blocking off extra}
t trailing-opt-query {fconfigure $f -blocking off -translation}
t junk-first {fconfigure $f extra}
t junk-pair {fconfigure $f extra junk}
t bogus-opt-pair {fconfigure $f -bogus x}
t query-then-junk {fconfigure $f -translation auto junk}
t two-sets {fconfigure $f -blocking on -buffering line; fconfigure $f -blocking -buffering}
close $f
