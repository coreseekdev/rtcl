# `proc` definition-time errors (Tcl 8.6 proc-old tests, probed on 8.6.17):
# - the arity usage is the standard `wrong # args: should be "proc name
#   args body"` (NOT the rtcl 4-argument statics form, which stays as an
#   extension but only for standard-shaped calls);
# - the formal-parameter list is parsed as a STRICT list: a broken brace
#   yields the list error `unmatched open brace in list`
#   (TCL VALUE LIST BRACE);
# - a malformed formal element reports the argument-format message with
#   errorCode `TCL OPERATION PROC FORMALARGUMENTFORMAT`.
proc gv {v} { if {[catch {uplevel 1 [list set $v]} r]} { return UNDEF } ; return $r }
proc t {label script} {
    unset -nocomplain ::errorCode
    set c [catch {uplevel 1 $script} m]
    puts "$label|$c|$m|[gv ::errorCode]"
}
t 1 {proc}
t 2 {proc p}
t 3 {proc p args}
t 4 {proc tproc \{xyz {return foo}}
t 5 {proc p {{x}} {body}}
t 6 {proc p {x 1 2} {body}}
