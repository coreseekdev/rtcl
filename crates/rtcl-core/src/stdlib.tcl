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

# ── auto_qualify (tclsh 8.6 init.tcl, verbatim port) ────────────────────

# auto_qualify — Fully-qualified command-name candidates for auto lookup.
# Decoded semantics (tclsh 8.6.17, gen_init-1.x):
#   1. Runs of >=2 colons collapse to `::`; n = substitution count
#      (`:::foo::::bar` -> `::foo::bar`, n=2).
#   2. Leading `::` names: n>1 returns the normalized name alone
#      (already global-qualified); n<=1 returns the tail after `::`
#      (`::global` -> `global` — a bare global name needs no candidates).
#   3. Unqualified names: n=0 (no separators) -> current-namespace
#      candidate + bare name (skipped when the namespace IS `::`);
#      n>0 -> current-namespace candidate + `::`-qualified global
#      candidate (plain `::$cmd` when the namespace IS `::`).
# Ported from tcl/library/init.tcl (Tcl 8.6) — auto_load/unknown rely on
# this exact candidate list.
proc auto_qualify {cmd namespace} {
    # count separators and clean them up
    # (making sure that foo:::::bar will be treated as foo::bar)
    set n [regsub -all {::+} $cmd :: cmd]

    # Ignore namespace if the name starts with ::
    # Handle special case of only leading ::

    if {[string match ::* $cmd]} {
        if {$n > 1} {
            # (::foo::bar , *) -> ::foo::bar
            return [list $cmd]
        } else {
            # (::global , *) -> global
            return [list [string range $cmd 2 end]]
        }
    }

    # Potentially returning 2 elements to try  :
    # (if the current namespace is not the global one)

    if {$n == 0} {
        if {$namespace eq "::"} {
            # (nocolons , ::) -> nocolons
            return [list $cmd]
        } else {
            # (nocolons , ::sub) -> ::sub::nocolons nocolons
            return [list ${namespace}::$cmd $cmd]
        }
    } elseif {$namespace eq "::"} {
        # (foo::bar , ::) -> ::foo::bar
        return [list ::$cmd]
    } else {
        # (foo::bar , ::sub) -> ::sub::foo::bar ::foo::bar
        return [list ${namespace}::$cmd ::$cmd]
    }
}

# ::tcl::tm — Tcl Modules (port of tcl 8.6 library/tm.tcl, trimmed to the
# command surface rtcl exposes; the corpus pins path/roots existence, the
# ensemble dispatch errors, and add/remove-with-no-args being silent no-ops
# — tm-1.1..2.1).
#
# tclsh loads tm.tcl lazily through the auto_loader: a fresh interpreter
# has NO ::tcl::tm commands and `catch {::tcl::tm::path}` triggers the
# load (then the catch swallows the no-subcommand error).  rtcl has no
# filesystem auto_load, so the suite is defined eagerly — every corpus
# case observes only the post-load state.
namespace eval ::tcl::tm {
    # Default search paths for modules.  rtcl is self-contained (wasm):
    # none.
    variable paths {}

    # The regex pattern a file name has to match to make it a Tcl Module.
    # (Kept for shape parity; used only by the package-unknown handler,
    # which rtcl does not install.)
    variable pkgpattern {^([_[:alpha:]][:_[:alnum:]]*)-([[:digit:]].*)[.]tm$}

    # Export the public API (tclsh: ensemble on `path` with exactly the
    # add/remove/list subcommands; `path foo` → "unknown or ambiguous
    # subcommand \"foo\": must be add, list, or remove").
    namespace export path
    namespace ensemble create -command path -subcommands {add remove list}
}

# ::tcl::tm::add — prepend module search paths (PART OF THE ::tcl::tm::path
# ENSEMBLE).  A path already on the list, or empty, is silently ignored; a
# path that is an ancestor/descendant of an existing one errors.
proc ::tcl::tm::add {args} {
    variable paths

    set newpaths $paths
    foreach p $args {
        if {($p eq "") || ($p in $newpaths)} {
            # Ignore any path which is empty or already on the list.
            continue
        }

        # Search for paths which are subdirectories of the new one: the
        # new path must not be an ancestor of an existing one.
        set pos [lsearch -glob $newpaths ${p}/*]
        if {$pos >= 0} {
            return -code error \
                "$p is ancestor of existing module path [lindex $newpaths $pos]."
        }

        # Existing paths which are ancestors of the new one.
        foreach ep $newpaths {
            if {[string match ${ep}/* $p]} {
                return -code error \
                    "$p is subdirectory of existing module path $ep."
            }
        }

        set newpaths [linsert $newpaths 0 $p]
    }

    set paths $newpaths
    return
}

# ::tcl::tm::remove — drop paths from the list (PART OF THE
# ::tcl::tm::path ENSEMBLE); silently ignores unknown paths.
proc ::tcl::tm::remove {args} {
    variable paths

    foreach p $args {
        set pos [lsearch -exact $paths $p]
        if {$pos >= 0} {
            set paths [lreplace $paths $pos $pos]
        }
    }
}

# ::tcl::tm::list — the search path (PART OF THE ::tcl::tm::path ENSEMBLE).
proc ::tcl::tm::list {} {
    variable paths
    return  $paths
}

# ::tcl::tm::roots — derive module search paths from root directories
# (tclsh: for each root, tcl$major/{major.n, ..., site-tcl} added via
# `path add`).  Only the command's existence is pinned (tm-2.1); the
# filesystem walk is inert here.
proc ::tcl::tm::roots {paths} {
    foreach pa $paths {
        set p [file join $pa tcl8]
        path add $p
    }
    return
}
