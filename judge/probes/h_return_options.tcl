# Tcl 8.6 `return` option parsing (TclProcessReturn / TclMergeReturnOptions,
# generic/tclCmdMZ.c), probed on tclsh 8.6.17:
# - explicitResult = (objc % 2 == 0) counting the `return` word itself, so an
#   ODD number of trailing words ends with the explicit result;
# - unknown option KEYS are silently ignored (`return -badOption foo message`
#   completes with code 2 carrying "message");
# - a later pair overwrites an earlier one; -options {dict} merges in place
#   and its value must be an even-length list (`expected dict but got "X"`,
#   TCL RESULT ILLEGAL_OPTIONS);
# - bad -code/-level/-errorcode values raise TCL RESULT ILLEGAL_CODE /
#   ILLEGAL_LEVEL / ILLEGAL_ERRORCODE; integer codes go through Tcl's
#   integer parser (hex 0x2 ok, 3000000000 truncates to the C int).
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
t 1 {return -badOption foo message}
t 2 {return -code}
t 3 {return -code nosuch x}
t 4 {return -level}
t 5 {return -level x}
t 6 {return -level -1 x}
t 7 {return -level 0 x}
t 8 {proc p {} {return -level 0 -code ok direct}; p}
t 9 {proc p {} {return -options {-code break} x}; set r [catch p m]; list $r $m}
t 10 {proc p {} {return -options {-code error -errorcode {A B}} msg}; set r [catch p m]; list $r $m [gv ::errorCode]}
t 11 {return -options {-code} x}
t 12 {return -options {onlyone} x}
t 13 {return -errorcode}
t 14 {return -errorcode {not a list}}
t 15 {return -errorcode "a \{b"}
t 16 {return -code 0x2}
t 17 {return -code 3000000000}
t 18 {return -code 1x}
t 19 {proc p {} {return -code error -errorcode {X Y} msg}; set r [catch p m]; list $r $m [gv ::errorCode]}
t 20 {proc p {} {return -code error -errorinfo custom msg}; set r [catch p m]; unset -nocomplain ::errorCode; set ::errorInfo}
