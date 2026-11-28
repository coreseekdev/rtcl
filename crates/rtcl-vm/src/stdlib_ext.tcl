# rtcl stdlib — std-only extension procs (36 项扩展裁定为 std-only 的
# Tcl 侧部分)。load_stdlib 仅在 feature "ext" 下评估本文件；Core
# Profile（wasm core 轨，ext 关）不加载——ensemble/popen/stackdump/
# errorInfo/{file copy}/{file delete force} 在 core 轨是 unknown
# command。lambda/curry/function/ref 家族是 core-in（留在 stdlib.tcl）。
# 原文与注释自 stdlib.tcl 拆出（2026-11-28，D6 Core Profile）。

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

# ── popen (jimtcl tclcompat.tcl) ────────────────────────────────────────

# popen — Open a pipe to/from a command.
# Uses the native `open |command ?mode?` pipe channel support.
proc popen {cmd {mode "r"}} {
    open |$cmd $mode
}

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
