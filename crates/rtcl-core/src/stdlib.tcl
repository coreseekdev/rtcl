# rtcl standard library — Tcl-level command extensions
# Embedded at compile time via include_str!() and evaluated during Interp::new().
# Modeled after jimtcl's tclcompat.tcl.

# throw — Generate an exception with the given code and optional message.
# jimtcl implements this in tclcompat.tcl.
proc throw {code {msg ""}} {
    return -code $code $msg
}

# parray — Pretty-print an array.
# jimtcl implements this in tclcompat.tcl.
proc parray {arrayname {pattern *}} {
    upvar $arrayname a
    set max 0
    foreach name [array names a $pattern] {
        if {[string length $name] > $max} {
            set max [string length $name]
        }
    }
    incr max [string length $arrayname]
    incr max 2
    foreach name [lsort [array names a $pattern]] {
        puts [format "%-${max}s = %s" ${arrayname}($name) $a($name)]
    }
}
