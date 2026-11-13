# chan copy: same-encoding copies take the byte fast path (-size limits
# bytes, may split a multibyte sequence, output translation/encoding do
# not apply), cross-encoding copies count characters.  The -command form
# is a background copy: scheduling has no visible effect, update/vwait
# pumps it, the callback receives the cumulative total, both ends are
# busy meanwhile, close aborts silently.
set d /tmp
proc t {label script} {
    if {![info exists ::errorCode]} {set ::errorCode NONE}
    set c [catch {uplevel 1 $script} r]
    puts "$label|$c|$r|EC=$::errorCode"
}
set h [open $d/rtcl_cp_u8.txt w]; fconfigure $h -encoding utf-8; puts -nonewline $h "héllo"; close $h
set h [open $d/rtcl_cp_lat.txt w]; fconfigure $h -encoding iso8859-1; puts -nonewline $h "héllo"; close $h
set h [open $d/rtcl_cp_big.txt w]; puts -nonewline $h [string repeat x 20000]; close $h
t same-cut {set a [open $d/rtcl_cp_u8.txt r]; set b [open $d/rtcl_cp_o1.txt w]; set n [chan copy $a $b -size 3]; close $a; close $b; set f [open $d/rtcl_cp_o1.txt r]; fconfigure $f -encoding binary; set s [read $f]; close $f; list $n [binary encode hex $s]}
t same-full {set a [open $d/rtcl_cp_u8.txt r]; set b [open $d/rtcl_cp_o2.txt w]; set n [chan copy $a $b]; close $a; close $b; set n}
t lat-cut {set a [open $d/rtcl_cp_lat.txt r]; fconfigure $a -encoding iso8859-1; set b [open $d/rtcl_cp_o3.txt w]; fconfigure $b -encoding iso8859-1; set n [chan copy $a $b -size 2]; close $a; close $b; set f [open $d/rtcl_cp_o3.txt r]; fconfigure $f -encoding binary; set s [read $f]; close $f; list $n [binary encode hex $s]}
t cross {set a [open $d/rtcl_cp_lat.txt r]; fconfigure $a -encoding iso8859-1; set b [open $d/rtcl_cp_o4.txt w]; fconfigure $b -encoding utf-8; set n [chan copy $a $b -size 3]; close $a; close $b; set f [open $d/rtcl_cp_o4.txt r]; fconfigure $f -encoding binary; set s [read $f]; close $f; list $n [binary encode hex $s]}
t bin-pair {set a [open $d/rtcl_cp_lat.txt r]; fconfigure $a -encoding binary; set b [open $d/rtcl_cp_o5.txt w]; fconfigure $b -encoding binary; set n [chan copy $a $b -size 3]; close $a; close $b; set f [open $d/rtcl_cp_o5.txt r]; fconfigure $f -encoding binary; set s [read $f]; close $f; list $n [binary encode hex $s]}
t dir-err {set c [catch {chan copy stdout stdin} m]; list $c $m}
proc cbc {x} {lappend ::evs $x}
t bg-schedule {set ::evs {}; set a [open $d/rtcl_cp_big.txt r]; set b [open $d/rtcl_cp_o6.txt w]; set n [chan copy $a $b -command cbc]; set r [list n=$n eof=[eof $a] sz=[file size $d/rtcl_cp_o6.txt] evs=$::evs]; close $a; close $b; set r}
t bg-update {set ::evs {}; set a [open $d/rtcl_cp_big.txt r]; set b [open $d/rtcl_cp_o7.txt w]; chan copy $a $b -size 10 -command cbc; update; set r [list eof=[eof $a] evs=$::evs sz=[file size $d/rtcl_cp_o7.txt]]; close $a; close $b; set r}
t bg-vwait {set ::evs {}; set ::d 0; set a [open $d/rtcl_cp_big.txt r]; set b [open $d/rtcl_cp_o8.txt w]; chan copy $a $b -command cbc; after 200 {set ::d 1}; vwait ::d; set r [list evs=$::evs sz=[file size $d/rtcl_cp_o8.txt]]; close $a; close $b; set r}
t bg-busy {set ::evs {}; set a [open $d/rtcl_cp_big.txt r]; set b [open $d/rtcl_cp_o9.txt w]; chan copy $a $b -command cbc; set g [catch {gets $a} m1]; set p [catch {puts $b hi} m2]; set sk [catch {seek $a 0} m3]; set tl [catch {tell $a} m4]; set fc [catch {fconfigure $a -encoding iso8859-1} m5]; set cl [catch {close $a} m6]; update; set evs=$::evs; set r [list gets=$g/$m1 puts=$p/$m2 seek=$sk/$m3 tell=$tl/$m4 fcfg=$fc/$m5 close=$cl/$m6 evs=]; close $b; set r}
