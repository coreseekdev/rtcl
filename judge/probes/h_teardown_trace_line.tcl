# proc frame teardown + absolute line numbers (Tcl 8.6, probed on 8.6.17):
# - `trace add variable` unset traces registered on proc LOCALS fire when
#   the frame dies (any completion, error unwinding included), AFTER the
#   values are gone (info exists -> 0 inside the callback), and a trace
#   callback error stays a background error that leaves the in-flight
#   errorInfo untouched;
# - the `(procedure ... line N)` frame for an error inside a braced `while`
#   body uses the enclosing script's absolute line table (tclsh compiles
#   loop bodies inline); a variable-sourced body keeps its own numbering.
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
    set ::log {}
    set c [catch {uplevel 1 $script} m]
    puts "$label|$c|$m|[set ::log]|[gv ::errorCode]|[nle [gv ::errorInfo]]"
}
t 1 {proc tp {n i op} {lappend ::log "$n/$i/$op/[info exists n]"}; proc u x {trace add variable x unset tp; set x val}; u 1}
t 2 {proc tp {n i op} {lappend ::log "$n/$i/$op/[info exists n]"}; proc u x {trace add variable x unset tp; error boom}; catch {u 1}}
t 3 {proc tp {n i op} {error tracefail}; proc u x {trace add variable x unset tp}; catch {u 1}; unset -nocomplain ::errorCode}
t 4 {proc w {} {
while 1 {
    error inbody
}
}; catch w; set ::errorInfo}
