if {![info exists ::errorCode]} {set ::errorCode NONE}
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r|EC=$::errorCode" }
t puts0 {chan puts}
t puts1 {chan puts stdout}
t puts4 {chan puts stdout a b}
t read0 {chan read}
t read3 {chan read stdout 1 2}
t seek0 {chan seek}
t seek1 {chan seek stdout}
t seek4 {chan seek stdout 1 start x}
t cfg0 {chan configure}
t cfg1 {chan configure stdout -buffering line extra}
t gets0 {chan gets}
t gets2 {chan gets stdout a b}
t flush-extra {chan flush stdout x}
t tell-extra {chan tell stdout x}
t eof-extra {chan eof stdout x}
t blocked-extra {chan blocked stdin x}
t ev0 {chan event}
t ev1 {chan event stdin}
t ev4 {chan event stdin readable {} x}
t pop2 {chan pop stdout write}
t postevent1 {chan postevent stdout}
t fcfg-block {set f [open /tmp/rtcl_probe.txt w]; set r [catch {fconfigure $f -block} m]; close $f; list $r $m}
t copy1 {chan copy stdout}
t copy4 {chan copy stdout stdin -size 1 x}
t copy-size-noint {catch {chan copy stdout stdin -size zz} m; set m}
t copy-dir {catch {chan copy stdin stdout} m; set m}
t copy-2read {set f [open /tmp/rtcl_probe.txt r]; set g [open /tmp/rtcl_probe2.txt w]; set r [catch {chan copy $f $g} m]; close $f; close $g; list $r $m}
