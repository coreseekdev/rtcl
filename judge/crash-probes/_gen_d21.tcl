# Writes judge/crash-probes/d21_interp_family.tcl from _d21_source.tcl:
# every check of the source runs under this tclsh, and its output becomes
# the expected value.  Run from the repository root:
#   tclsh judge/crash-probes/_gen_d21.tcl
#
# Checks whose result is not a portable expectation are skipped, by tag
# (SKIP below): hash-table order, the machine's files and sockets, and the
# places rtcl deliberately differs from tclsh (listed in the probe's head).
source [file join [file dirname [info script]] _chk.tcl]

set dir [file dirname [info script]]
set f [open [file join $dir _d21_source.tcl]]
set lines [split [read -nonewline $f] \n]
close $f

set SKIP {
    # hash-table order
    p-slaves h-hidden h-hidden-token h-hidden-after h-invoke-interp x-7 x-9
    # tclsh's `unknown` handler recurses to its limit once `set` is hidden;
    # rtcl reports the command as unknown
    h-hidden-set h-hidden-set-ec h-hide-interp h-compiled-after
    a-hidden-target a-call-into-hidden
    # rtcl deletes no interpreter that is, or holds one that is, being
    # evaluated
    d-inflight d-inflight2 a-renamed-slaves x-11 x-12 x-13
    # rtcl's own command set: `auto_*`, `job`, `wait` hidden; no `tcl:file:*`
    # internals; `min`/`max` are built in, not aliases
    s-hidden-common s-aliases s-aliases-self s-hidden-self
    # the machine's files
    s-open s-file s-source s-socket
    # the command a wrong-args message names for an alias (tclsh is not
    # consistent itself), and TIP 356 stacks
    a-parent-into-child-wa a-bad-target-arg a-error-stack
}

# Record in place of the source's T and C: the same evaluation each does.
# `items` is the probe's body in the making — a plain line of the source
# as it is, a check as its chk line.
set ::items {}
proc T {tag script} {
    set ::errorCode NONE
    set c [catch {uplevel 1 $script} r]
    set ec [expr {[info exists ::errorCode] ? $::errorCode : {}}]
    if {[lsearch -exact $::SKIP $tag] >= 0} {
        # Still part of the state the later checks sit on: run, unchecked.
        lappend ::items "U [list $script]"
        return
    }
    lappend ::items "chk $tag \[T [list $script]\] [list [list $c $r $ec]]"
}
proc C {tag path script} {
    set ::errorCode NONE
    catch {interp eval $path {proc P {script} {
        set c [catch {uplevel 1 $script} r o]
        set e [catch {dict get $o -errorcode} e]
        set i [catch {dict get $o -errorinfo} i]
        list $c $r [expr {$e ? "" : $e}] [expr {$i ? "" : [string map {"\n" ^} $i]}]
    }}}
    set c [catch {interp eval $path [list P $script]} r]
    set ec [expr {[info exists ::errorCode] ? $::errorCode : {}}]
    set script [concat [list interp eval $path] [list [list P $script]]]
    if {![info exists ::p_paths($path)]} {
        # The helper, in that interpreter, before its first use.
        set ::p_paths($path) 1
        set def "proc P \x7bscript\x7d \x7b\n[string trimright $::pbody]\n\x7d"
        lappend ::items "interp eval $path [list $def]"
    }
    if {[lsearch -exact $::SKIP $tag] >= 0} {
        lappend ::items "U [list $script]"
        return
    }
    lappend ::items "chk $tag \[T [list $script]\] [list [list $c $r $ec]]"
}

# The body of the helper the child-side checks go through.
set pbody {}
set in_p 0
foreach line $lines {
    if {$in_p} {
        if {$line eq "\}"} break
        append pbody $line \n
    } elseif {[string match "proc P {script} *" $line]} {
        set in_p 1
    }
}
set ::pbody $pbody

# The whole source runs, in order: the plain lines build the state the
# checks sit on (interpreters created, commands hidden) and go into the
# probe as they are; T and C above do the checks.
set in_proc 0
foreach line $lines {
    # The helper's definition is emitted separately.
    if {$in_proc} {
        if {$line eq "\}"} { set in_proc 0 }
        continue
    }
    if {[string match "proc *" $line]} { set in_proc 1; continue }
    if {[string trim $line] eq "" || [string match "#*" $line] || $line eq "puts DONE"} continue
    if {![regexp {^(T|C) } $line]} {
        uplevel #0 $line
        lappend ::items $line
    } else {
        uplevel #0 $line
    }
}


set out [open [file join $dir d21_interp_family.tcl] w]
puts $out [string trim {
# crash-probe: d21 the `interp` command family
# Every tclsh form of `interp` was a `wrong # args` or "not supported";
# a child interpreter came up with every command its parent has (`exec`
# included), so `interp create -safe` made nothing safe; `interp hide`
# only took the name out of `info commands`; aliases into the master were
# misaddressed and a loop of aliases across interpreters had no end.
#
# Expected values are tclsh 8.6.17's own outputs, check by check, in the
# order the state is built: _gen_d21.tcl writes this file from
# _d21_source.tcl.  Where rtcl deliberately differs from tclsh the check
# is left out (the generator's SKIP list names each):
#   - rtcl sorts `interp slaves` / `interp hidden` (tclsh: hash order);
#   - tclsh's `unknown` handler recurses to the limit once `set` is
#     hidden in a child; rtcl says `invalid command name "set"`;
#   - rtcl deletes no interpreter that is, or holds one that is, being
#     evaluated;
#   - rtcl's own command set: `auto_*`, `job`, `wait` hidden in a safe
#     interpreter, `::tcl::mathfunc::min`/`max` built in, not aliases.
source [file join [file dirname [info script]] _chk.tcl]
proc T {script} {
    # A success leaves the previous error's errorCode standing: start
    # each check from NONE.
    set ::errorCode NONE
    set c [catch {uplevel 1 $script} r]
    return [list $c $r [expr {[info exists ::errorCode] ? $::errorCode : {}}]]
}
# A skipped check still builds the state the later ones sit on: its
# script runs, unchecked.
proc U {script} {
    catch {uplevel 1 $script}
}
}]
puts $out ""
puts $out "set ::errorCode NONE"
puts $out "# The helper the checks inside a child go through (defined where it"
puts $out "# is first used): code, result, the child's errorCode, its errorInfo"
puts $out "# with newlines as ^."
puts $out ""
foreach item $::items {
    puts $out $item
}
set n [llength $::items]
puts $out ""
puts $out "# $n lines, the chk ones are the checks."
close $out
puts "d21: wrote $n checks"
