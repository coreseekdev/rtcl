# rtcl standard library — Tcl-level command extensions
# Embedded at compile time via include_str!() and evaluated during Interp::new().
# Ported from jimtcl's stdlib.tcl, tclcompat.tcl, ensemble.tcl.

# ── throw / parray (already implemented) ────────────────────────────────

# throw — Generate an exception with the given code and optional message.
# jimtcl: tclcompat.tcl
proc throw {code {msg ""}} {
    return -code $code $msg
}

# parray — Pretty-print an array.
# jimtcl: tclcompat.tcl
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

# ── Phase 5B — function / lambda / curry ────────────────────────────────

# function — Returns its argument. Useful with `local`:
#   local function [lambda ...]
# jimtcl: stdlib.tcl
proc function {value} {
    return $value
}

# lambda — Create an anonymous procedure.
# jimtcl: stdlib.tcl (uses ref for unique names)
proc lambda {arglist args} {
    set name [ref {} function lambda.finalizer]
    proc $name $arglist {*}$args
    return $name
}

proc lambda.finalizer {name val} {
    rename $name {}
}

# curry — Like alias, but creates and returns an anonymous procedure.
# jimtcl: stdlib.tcl
proc curry {args} {
    set name [ref {} function lambda.finalizer]
    alias $name {*}$args
    return $name
}

# ── defer ───────────────────────────────────────────────────────────────

# defer — Now implemented natively in Rust (cmd_defer in introspect.rs).
# The native version pushes scripts onto the frame's deferred_scripts list,
# which are executed in reverse order when the proc exits.

# ── loop ────────────────────────────────────────────────────────────────

# loop — Enhanced for loop. Now implemented natively in Rust (loops.rs).
# Signature: loop var ?first? limit ?increment? body
# The native implementation supports the 3-arg form (loop var limit body).

# ── dict update / dict getdef ───────────────────────────────────────────

# dict update — Script-based implementation.
# jimtcl: stdlib.tcl
# Note: rtcl already has native dict update via dict.rs, but this provides
# the Tcl-level fallback if needed. We skip defining it if it already exists.

# dict getdef — Get a value from a dict with a default if key doesn't exist.
# Not in jimtcl but common in Tcl 8.7+.
proc {dict getdef} {dictionary args} {
    if {[llength $args] < 2} {
        return -code error "wrong # args: should be \"dict getdef dictionary ?key ...? key default\""
    }
    set default [lindex $args end]
    set keys [lrange $args 0 end-1]
    if {[dict exists $dictionary {*}$keys]} {
        return [dict get $dictionary {*}$keys]
    }
    return $default
}

# ── ensemble ─────────────────────────────────────────────────────────────

# ensemble — Create an ensemble command that dispatches to subcommands.
# jimtcl: ensemble.tcl (adapted for rtcl, without statics support)
proc ensemble {command args} {
    set autoprefix "$command "
    set badopts "should be \"ensemble command ?-automap prefix?\""
    if {[llength $args] % 2 != 0} {
        return -code error "wrong # args: $badopts"
    }
    foreach {opt value} $args {
        switch -- $opt {
            -automap { set autoprefix $value }
            default { return -code error "wrong # args: $badopts" }
        }
    }
    # Build the proc body with substituted autoprefix
    set body [format {
        set target "%s$subcmd"
        tailcall $target {*}$args
    } $autoprefix]
    proc $command {subcmd args} $body
}

# ── glob (requires readdir) ─────────────────────────────────────────────
# rtcl already has a native glob command, so we don't need the Tcl version.

# ── file copy ────────────────────────────────────────────────────────────

# {file copy} — Copy a file using open/read/close.
# jimtcl: tclcompat.tcl
proc {file copy} {args} {
    set force 0
    set source ""
    set target ""
    foreach arg $args {
        if {$arg eq "-force"} {
            set force 1
        } elseif {$source eq ""} {
            set source $arg
        } else {
            set target $arg
        }
    }
    if {$source eq "" || $target eq ""} {
        return -code error "wrong # args: should be \"file copy ?-force? source target\""
    }
    if {!$force && [file exists $target]} {
        return -code error "error copying \"$source\" to \"$target\": file already exists"
    }
    set in [open $source r]
    set data [read $in]
    close $in
    set out [open $target w]
    puts -nonewline $out $data
    close $out
}

# ── file delete -force ──────────────────────────────────────────────────

# {file delete force} — Recursive directory deletion.
# jimtcl: tclcompat.tcl (requires readdir)
proc {file delete force} {path} {
    if {[file isdirectory $path]} {
        foreach e [readdir $path] {
            {file delete force} $path/$e
        }
    }
    file delete $path
}

# ── fileevent (shim) ────────────────────────────────────────────────────

# fileevent — Compatibility shim. Not needed in rtcl.
# jimtcl: tclcompat.tcl
proc fileevent {args} {
    tailcall {*}$args
}

# ── stackdump / errorInfo ───────────────────────────────────────────────

# stackdump — Human-readable stack trace formatter.
# jimtcl: stdlib.tcl
proc stackdump {stacktrace} {
    set lines {}
    lappend lines "Traceback (most recent call last):"
    foreach {cmd l f p} [lreverse $stacktrace] {
        set line {}
        if {$f ne ""} {
            append line "  File \"$f\", line $l"
        }
        if {$p ne ""} {
            append line ", in $p"
        }
        if {$line ne ""} {
            lappend lines $line
            if {$cmd ne ""} {
                lappend lines "    $cmd"
            }
        }
    }
    if {[llength $lines] > 1} {
        return [join $lines \n]
    }
}

# errorInfo — Format error info from stack trace.
# jimtcl: stdlib.tcl
proc errorInfo {msg {stacktrace ""}} {
    if {$stacktrace eq ""} {
        set stacktrace [stacktrace]
    }
    set result "$msg\n"
    set dump [stackdump $stacktrace]
    if {$dump ne ""} {
        append result $dump
    }
    string trim $result
}

# ── namespace inscope ────────────────────────────────────────────────────

# namespace inscope — Evaluate a script in a namespace context.
# jimtcl: nshelper.tcl
proc {namespace inscope} {ns args} {
    tailcall namespace eval $ns $args
}

# ── json::encode / json::decode ──────────────────────────────────────────

# Now implemented natively in Rust (commands/json.rs).
# Registered as: json::decode, json::encode, json (ensemble).

# ── popen (jimtcl tclcompat.tcl) ────────────────────────────────────────

# popen — Open a pipe to/from a command.
# Uses the native `open |command ?mode?` pipe channel support.
proc popen {cmd {mode "r"}} {
    open |$cmd $mode
}
