# `info object call` / `info class call` chain descriptors (Tcl 8.6 tclOO,
# probed on 8.6.17): each entry is a 4-element list
#   method entry : {method NAME OWNER kind}   — kind is `method` or
#                  `core method: "B"` (rendered braced by the outer list)
#   filter entry : {filter NAME OWNER methodkind}
#   unknown hit  : {unknown unknown ::oo::object {core method: "unknown"}}
# Exactly two arguments; a non-object name is TCL LOOKUP OBJECT, a
# non-class owner of `info class call` is TCL LOOKUP CLASS.
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
    set c [catch {uplevel 1 $script} m]
    puts "$label|$c|$m|[gv ::errorCode]"
}
oo::class create A {method m1 {} {}}
A create a1
oo::class create B {method m2 {} {}; filter f1; method f1 args {next}}
B create b1
oo::define B mixin A
t 1 {info object call}
t 2 {info object call a1}
t 3 {info object call a1 m1 extra}
t 4 {info object call a1 m1}
t 5 {info object call notanobject m1}
t 6 {info class call}
t 7 {info class call A}
t 8 {info class call A m1}
t 9 {info class call a1 m1}
t 10 {info class call notanobject m1}
t 11 {info object call b1 m2}
t 12 {info object call b1 nometh}
t 13 {info object call a1 nothere}
t 14 {info object call ::oo::object destroy}
t 15 {info object call ::oo::object nometh}
