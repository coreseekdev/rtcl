# Writes a crash probe whose expected values are tclsh's own outputs.
# Run from the repository root, with the tclsh that is the oracle:
#   tclsh judge/crash-probes/_gen.tcl <source.tcl> <probe.tcl>
#
# The source is the checks in the order their state is built up:
#   T <tag> {<script>}            — here: code, result, errorCode
#   C <tag> <path> {<script>}     — in a child interpreter: code, result,
#                                  the child's errorCode, its errorInfo
#   # skip <tag> <tag> … -- why   — these checks run (the later ones sit
#                                  on their state) but are not asserted:
#                                  what they produce is not a portable
#                                  expectation, or rtcl differs on purpose
#   plain lines                   — state; they go into the probe as they are
# The probe keeps the comment block at the top of the file it replaces,
# so the head (what defect, what is left out and why) is written once.
# The probe's own helpers (T, U, and P where a C check needs it) are
# emitted after that head; see the d21 and d22 probes.
#
# A listing's order depends on how full the tables are, so a value
# recorded while this script's helpers sit in the global table need not
# hold in the probe, whose state is leaner.  After generating, run the
# second mode: it writes every expected value over again from the probe's
# OWN lines, run under a fresh tclsh —
#   tclsh _gen.tcl --refix <probe.tcl>
# Everything of this mode sits in its own namespace: nothing it keeps
# may show up in a listing the probe asks for.
namespace eval ::refix {
    # The parts of a probe's check line: `chk`, then the tag, then the
    # call `[T <script>]`, then the want.  The line is not a list (the
    # call word has brackets in it).  The script is one word —
    # brace-quoted, which `list` wrote balanced, or bare (`[T append]`).
    proc parts {line} {
        set t0 [string first "\[T " $line]
        set b0 [expr {$t0 + 3}]
        set n [string length $line]
        if {[string index $line $b0] ne "\{"} {
            set end [expr {[string first "\]" $line $b0] - 1}]
        } else {
            set depth 0
            set i $b0
            while {$i < $n} {
                set c [string index $line $i]
                if {$c eq "\\"} { incr i 2; continue }
                if {$c eq "\{"} { incr depth }
                if {$c eq "\}"} {
                    incr depth -1
                    if {$depth == 0} { break }
                }
                incr i
            }
            set end $i
        }
        set tag [lindex [string range $line 4 [expr {$t0 - 1}]] 0]
        list $tag [string range $line $t0 [expr {$end + 1}]] \
            [string range $line $b0 $end] [string range $line [expr {$end + 2}] end]
    }

    proc run {probe} {
        set f [open $probe]
        variable lines [split [read -nonewline $f] \n]
        close $f
        set out {}
        set block {}
        foreach line $lines {
            if {![llength $block]} {
                if {[string match "source *" $line] || $line eq "" || [string match "#*" $line]} {
                    # Copied, never run: the probe's own `source` line
                    # would define the real `chk`.
                    lappend out $line
                    continue
                }
                if {[string match "chk *" $line] && [info complete $line]} {
                    lappend out [checked $line]
                    continue
                }
            }
            lappend block $line
            if {![info complete [join $block \n]]} { continue }
            set body [join $block \n]
            set block {}
            if {[string match "chk *" $body]} {
                # A check whose want runs over several lines (an
                # errorInfo in it).
                lappend out [checked $body]
            } else {
                uplevel #0 $body
                lappend out $body
            }
        }
        # Written through a sibling: a failure midway leaves the probe as
        # it was.
        set tmp "$probe.tmp"
        set f [open $tmp w]
        puts $f [join $out \n]
        close $f
        file rename -force $tmp $probe
        puts "$probe: expected values rewritten from its own run under tclsh"
    }

    # `chk <tag> [T {script}] <want>`, with the want replaced by what the
    # script gives here.
    proc checked {line} {
        lassign [parts $line] tag call script
        set got [uplevel #0 "T $script"]
        return "chk [list $tag] $call [list $got]"
    }
}

if {[lindex $argv 0] eq "--refix"} {
    ::refix::run [file join [file dirname [info script]] [lindex $argv 1]]
    exit 0
}
source [file join [file dirname [info script]] _chk.tcl]

lassign $argv source_name probe_name
if {$probe_name eq ""} {
    puts stderr "usage: tclsh _gen.tcl <source.tcl> <probe.tcl>"
    exit 2
}
set dir [file dirname [info script]]
set source [file join $dir $source_name]
set probe [file join $dir $probe_name]
set f [open $source]
set lines [split [read -nonewline $f] \n]
close $f

# The tags not asserted, with the source's own reasons.
set ::skip {}
foreach line $lines {
    if {[string match "# skip *" $line]} {
        # `# skip tag tag … -- why`
        set rest [string range $line 7 end]
        set cut [string first " -- " $rest]
        if {$cut >= 0} { set rest [string range $rest 0 [expr {$cut - 1}]] }
        if {[catch {lrange $rest 0 end} tags]} { set tags $rest }
        foreach tag $tags { lappend ::skip $tag }
    }
}

# Record in place of the source's T and C: the same evaluation each does.
# `items` is the probe's body in the making — a plain line of the source
# as it is, a check as its chk line.
set ::items {}
proc T {tag script} {
    set ::errorCode NONE
    set c [catch {uplevel 1 $script} r]
    set ec [expr {[info exists ::errorCode] ? $::errorCode : {}}]
    if {[lsearch -exact $::skip $tag] >= 0} {
        lappend ::items "U [list $script]"
        return
    }
    # A tag is one word whatever is in it (a tag with a blank in it — an
    # index with a blank — is still one tag).
    lappend ::items "chk [list $tag] \[T [list $script]\] [list [list $c $r $ec]]"
}
proc C {tag path script} {
    set ::errorCode NONE
    catch {interp eval $path [list proc P {script} $::pbody]}
    set c [catch {interp eval $path [list P $script]} r]
    set ec [expr {[info exists ::errorCode] ? $::errorCode : {}}]
    set script [concat [list interp eval $path] [list [list P $script]]]
    if {![info exists ::p_paths($path)]} {
        set ::p_paths($path) 1
        lappend ::items "interp eval $path \{proc P \{script\} \{\n[string trimright $::pbody]\n\}\}"
    }
    if {[lsearch -exact $::skip $tag] >= 0} {
        lappend ::items "U [list $script]"
        return
    }
    lappend ::items "chk [list $tag] \[T [list $script]\] [list [list $c $r $ec]]"
}

# The whole source runs, in order: the plain lines build the state the
# checks sit on and go into the probe as they are; T and C above do the
# checks.
set in_proc 0
foreach line $lines {
    # A procedure the source defines is part of the state too (and of the
    # probe), except the child-side helper, which the probe gets where it
    # is first used.
    if {$in_proc} {
        if {$line eq "\}"} {
            set in_proc 0
            lappend ::proc_block $line
            set body [join $::proc_block \n]
            unset ::proc_block
            # Defined here too, for the checks that follow.
            uplevel #0 $body
            if {!$::was_helper} { lappend ::items $body }
        } elseif {$::was_helper} {
            append ::pbody $line \n
        } else {
            lappend ::proc_block $line
        }
        continue
    }
    if {[string match "proc *" $line]} {
        if {[info complete $line]} {
            # A procedure defined on one line is an ordinary line.
            uplevel #0 $line
            lappend ::items $line
            continue
        }
        set in_proc 1
        set ::was_helper [string match "proc P {script} *" $line]
        set ::proc_block [list $line]
        continue
    }
    if {[string trim $line] eq "" || [string match "#*" $line] || $line eq "puts DONE"} continue
    uplevel #0 $line
    if {![regexp {^(T|C) } $line]} { lappend ::items $line }
}

# The head: the comment block the probe to be replaced starts with.
set head {}
if {[file exists $probe]} {
    set f [open $probe]
    foreach line [split [read -nonewline $f] \n] {
        if {[string match "#*" $line] || [string trim $line] eq ""} {
            lappend head $line
        } else {
            break
        }
    }
    close $f
}
while {[llength $head] && [string trim [lindex $head end]] eq ""} {
    set head [lrange $head 0 end-1]
}
if {![llength $head]} {
    set head [list "# crash-probe: $probe_name" \
        "# Generated by judge/crash-probes/_gen.tcl from $source_name: every" \
        "# expected value below is what tclsh gave for that check."]
}

set out [open $probe w]
puts $out [join $head \n]
puts $out ""
puts $out [string trim {
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
set ::errorCode NONE
}]
foreach item $::items { puts $out $item }
puts $out ""
puts $out "# [llength $::items] lines; the chk ones are the checks."
close $out
set n 0
foreach item $::items { if {[string match "chk *" $item]} { incr n } }
puts "$probe_name: wrote $n checks"
