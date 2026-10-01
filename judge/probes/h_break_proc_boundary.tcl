# break/continue crossing a proc boundary (Tcl 8.6, probed on 8.6.17):
# - a literal break that escapes the body is converted to an error AT the
#   proc boundary — errorInfo starts with the bare message (no command
#   position is logged) and the `(procedure "p" line 1)` frame always
#   reports line 1, errorCode `TCL RESULT UNEXPECTED`;
# - a caller's loop never sees it;
# - `return -code break/continue` instead propagates as a real
#   break/continue, carrying the return's value, and catching it does not
#   touch ::errorInfo.
proc gv {v} { if {[info exists $v]} { return [set $v] } ; return UNDEF }
# rtcl does not append the outermost catch-invocation frame
# (`invoked from within "uplevel 1 $script"`) to ::errorInfo; cut it
# so the probe locks the frames below it.
proc nle {s} {
    set m "\n    invoked from within\n\"uplevel 1 \$script\""
    set i [string first $m $s]
    if {$i >= 0} { set s [string range $s 0 [expr {$i - 1}]] }
    return [string map [list \n <nl>] $s]
}
proc t {label script} {
    unset -nocomplain ::errorCode
    unset -nocomplain ::errorInfo
    set c [catch {uplevel 1 $script} m]
    puts "$label|$c|$m|[gv ::errorCode]|[nle [gv ::errorInfo]]"
}
proc p {} {break}
proc q {} {continue}
proc r {} {return -code break carried}
proc s {} {return -code continue carried}
t 1 {p}
t 2 {q}
t 3 {foreach v {a b} {p; puts $v}; nomore}
t 4 {foreach v {a b} {q; puts $v}; nomore}
t 5 {foreach v {a b} {r; puts $v}; nomore}
t 6 {foreach v {a b} {s; puts $v}; nomore}
t 7 {set c [catch {r} m]; list $c $m}
t 8 {set c [catch {s} m]; list $c $m}
t 9 {proc inner {} {break}; proc outer {} {inner}; foreach v {x} {outer}; done}
