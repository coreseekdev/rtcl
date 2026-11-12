# rtcl standard library — Tcl-level command extensions
# Embedded at compile time via include_str!() and evaluated during Interp::new().
# Ported from jimtcl's stdlib.tcl, tclcompat.tcl, ensemble.tcl.

# ── throw / parray (already implemented) ────────────────────────────────

# throw — Generate an ERROR exception whose errorCode is the given
# code (tclsh 8.6 semantics: code is an arbitrary string/list riding
# ::errorCode — NOT a completion code; dict-deep 探针 `throw {E C} msg`
# → m=msg、errorCode=E C)。jimtcl 的 completion-code 形式弃用。
proc throw {code {msg ""} {info ""}} {
    if {[string length $info]} {
        return -code error -errorcode $code -errorinfo $info $msg
    }
    return -code error -errorcode $code $msg
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

# dict getdef (Tcl 8.7) intentionally absent — the oracle is tclsh
# 8.6.17, where it is an unknown subcommand (DIVERGENCES dict trio).

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

# tclsh exposes these as children of ::tcl.  The commands are registered
# natively (CMD_TABLE_ENSEMBLE), but the namespaces must exist for
# `info commands ::tcl::dict::*`, `namespace children ::tcl`, and
# `namespace which` to resolve qualified names.
namespace eval ::tcl::dict {}
namespace eval ::tcl::mathop {}
namespace eval ::tcl::unsupported {}

# ── tclsh init.tcl surfaces (canonical registration order, rtcl port) ──
# tclsh 8.6.17 defines these at startup in init.tcl; the iteration-order
# corpus needs the same names visible in the same :: table.  rtcl is
# self-contained (no filesystem auto_path scan): auto_load resolves only
# explicit auto_index entries, auto_execs mirrors the trimmed exec search.

proc tclLog {string} {
    puts stderr $string
}

# [auto_qualify] — exact port of init.tcl's auto_qualify (candidate
# names for a command in a namespace; two candidates when the namespace
# is not global).
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

    # Potentially returning 2 elements to try:
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

# [auto_load_index] — tclsh scans tclIndex files on $auto_path; rtcl is
# self-contained, so nothing is ever loaded (tclsh's "0" = not loaded).
proc auto_load_index {} {
    return 0
}

# [auto_load] — resolve via explicit auto_index entries only (tclsh's
# filesystem tclIndex scan doesn't apply to the self-contained rtcl).
proc auto_load {cmd {namespace {}}} {
    global auto_index auto_path
    if {$namespace eq ""} {
        set namespace [uplevel 1 [list ::namespace current]]
    }
    set nameList [auto_qualify $cmd $namespace]
    if {$cmd ni $nameList} {
        lappend nameList $cmd
    }
    foreach name $nameList {
        if {[info exists auto_index($name)]} {
            namespace inscope :: $auto_index($name)
            if {[namespace which -command $name] ne ""} {
                return 1
            }
        }
    }
    if {![info exists auto_path]} {
        return 0
    }
    return 0
}

# [auto_import] — invoked by [namespace import]; loads auto_index
# entries matching the pattern (rtcl: explicit entries only).
proc auto_import {pattern} {
    global auto_index
    foreach name [array names auto_index $pattern] {
        if {[namespace which -command $name] eq ""} {
            namespace inscope :: $auto_index($name)
        }
    }
}

# [auto_execs]/[auto_execok] — trimmed exec search over $env(PATH).
variable auto_execs_arr
proc auto_execok name {
    variable auto_execs_arr
    global env
    if {[info exists auto_execs_arr($name)]} {
        return $auto_execs_arr($name)
    }
    foreach dir [split [lindex [array get env PATH] 1] :] {
        if {$dir eq ""} { continue }
        set f [file join $dir $name]
        if {[file executable $f]} {
            set auto_execs_arr($name) [list $f]
            return $auto_execs_arr($name)
        }
    }
    set auto_execs_arr($name) ""
    return ""
}

# [history] — trimmed port of tclsh 8.6 library/history.tcl: the event
# list lives in ::tcl; add/event/info/keep are the observable surface.
namespace eval ::tcl {
    variable hist_events {}
    variable hist_keep 20
}
proc ::history {args} {
    # No subcommand: 'history info' (history.tcl's own preamble).
    if {![llength $args]} {
        set args info
    }
    tailcall ::tcl::history {*}$args
}
proc ::tcl::history {sub args} {
    variable hist_events
    variable hist_keep
    switch -- $sub {
        add {
            set ev [lindex $args 0]
            lappend hist_events $ev
            if {[llength $hist_events] > $hist_keep} {
                set hist_events [lrange $hist_events end-[expr {$hist_keep - 1}] end]
            }
        }
        event {
            set idx [lindex $args 0]
            if {[string index $idx 0] eq "-"} {
                set off [string range $idx 1 end]
                if {$off == 0} { return [lindex $hist_events end] }
                return [lindex $hist_events end-$off]
            }
            return [lindex $hist_events $idx]
        }
        info {
            set out {}
            set n [llength $hist_events]
            set from [expr {$n > $hist_keep ? $n - $hist_keep : 0}]
            for {set i $from} {$i < $n} {incr i} {
                lappend out [format "%6d  %s" $i [lindex $hist_events $i]]
            }
            return [join $out \n]
        }
        keep {
            set hist_keep [lindex $args 0]
        }
        clear {
            set hist_events {}
        }
        nextid {
            return [llength $hist_events]
        }
        change -
        redo {
            # Recorded-event editing has no interactive driver in rtcl.
        }
        default {
            return -code error "bad option \"$sub\": must be add, change, clear, event, info, keep, nextid, or redo"
        }
    }
}
