# seek/tell go through to the OS like tclsh: stdio redirected to a file
# or /dev/null is seekable, pipes/ttys report lseek's errno (ESPIPE);
# tell on a non-seekable channel returns -1 without raising.
set d /tmp
proc t {label script} {
    if {![info exists ::errorCode]} {set ::errorCode NONE}
    set c [catch {uplevel 1 $script} r]
    puts "$label|$c|$r|EC=$::errorCode"
}
t seek-stdin {seek stdin 0}
t tell-stdin {tell stdin}
t tell-stdout {tell stdout}
t seek-stdin-bad {set c [catch {seek stdin zz} m]; list $c $m}
t seek-neg {set f [open $d/rtcl_st.txt w]; puts $f "abc"; close $f; set f [open $d/rtcl_st.txt r]; set c [catch {seek $f -1} m]; close $f; list $c $m}
t seek-origin-bad {set f [open $d/rtcl_st.txt r]; set c [catch {seek $f 0 bogus} m]; close $f; list $c $m}
t seek-cycle {set f [open $d/rtcl_st.txt r]; read $f 2; set a [eof $f]; read $f 1; set b [eof $f]; read $f 1; set c [eof $f]; read $f 1; set e [eof $f]; set r2 [read $f 1]; seek $f 0; set d2 [eof $f]; set s [read $f 2]; close $f; list $a $b $c $e [string length $r2] $d2 $s}
t tell-file {set f [open $d/rtcl_st.txt r]; read $f 2; set p [tell $f]; close $f; set p}
