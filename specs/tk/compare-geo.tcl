# Compares two runs of specs/tk/geometry.tcl (real Tk vs rtcl).
#
#   tclsh specs/tk/compare-geo.tcl tk.out rtcl.out ?fontScale?
#
# Scaling per field: Tk reports canvas bounds in canvas units (logical),
# so the anchors are compared as-is; the text extents inside them and
# dlineinfo's numbers are Xft's device pixels, so they come down by
# `fontScale` (Xft.dpi / 96) before comparing with rtcl's 96 dpi values.
# Tolerances (logical px): dlineinfo x/y and canvas anchors within 1.5;
# text widths within half a pixel per glyph plus 4 %; linespace 1.5;
# baseline 2; entry/listbox reqwidth 5 % plus 3.

lassign $argv tkFile rtclFile scale
if {$rtclFile eq ""} {
    puts stderr "usage: compare-geo.tcl tk.out rtcl.out ?fontScale?"
    exit 2
}
if {$scale eq ""} { set scale 1.0 }

proc readRun {file} {
    set fh [open $file]
    set rows {}
    foreach line [split [read -nonewline $fh] \n] {
        set fields [split $line |]
        if {[llength $fields] != 3
                || [lindex $fields 0] ni {CB DL EW LW}} {
            continue
        }
        dict set rows [list [lindex $fields 0] [lindex $fields 1]] \
            [lindex $fields 2]
    }
    close $fh
    return $rows
}

set tk [readRun $tkFile]
set rtcl [readRun $rtclFile]
set checked 0
set bad {}

dict for {key expected} $tk {
    lassign $key kind name
    if {![dict exists $rtcl $key]} {
        lappend bad "missing in rtcl: $key"
        continue
    }
    set got [dict get $rtcl $key]
    if {$kind eq "DL" || $kind eq "CB"} {
        if {$expected eq "" || $got eq ""} {
            incr checked
            if {$expected ne $got} {
                lappend bad "$kind $name: tk=\"$expected\" rtcl=\"$got\""
            }
            continue
        }
        set et [split $expected]
        set gt [split $got]
        incr checked
        if {[llength $et] != [llength $gt]} {
            lappend bad "$kind $name: different shapes"
            continue
        }
        # Per-field tolerance: origins as-is, font-driven spans scaled.
        set i 0
        foreach a $et b $gt {
            set scaled [expr {$kind eq "CB" && $i < 2 ? 1.0 : $scale}]
            set want [expr {$a / $scaled}]
            if {$kind eq "CB" && $i >= 2} {
                set want [expr {($a - [lindex $et 0]) / $scaled + [lindex $gt 0]}]
            }
            if {$kind eq "CB" && $i == 3} {
                set want [expr {([lindex $et 3] - [lindex $et 1]) / $scale + [lindex $gt 1]}]
            }
            # Tk 8.6's dlineinfo reports the FIRST line's baseline for
            # every line (field 4 stays 20 while y walks 3/31/59): its own
            # quirk, so rtcl's y+ascent is not held to it.
            if {$kind eq "DL" && $i == 4} {
                incr i
                continue
            }
            set tol [expr {($kind eq "DL" && $i == 2)
                || ($kind eq "CB" && $i == 2)
                ? 0.5 * ($want / 8.0) + 0.04 * $want + 1.5
                : 1.5}]
            if {abs($b - $want) > $tol + 1e-9} {
                lappend bad [format "%s %s field %d: tk=%s rtcl=%s, want %.1f off %.1f > %.1f" \
                    $kind $name $i [lindex $et $i] $b $want [expr {abs($b - $want)}] $tol]
            }
            incr i
        }
    } else {
        incr checked
        set want [expr {$expected / double($scale)}]
        set tol [expr {0.05 * $want + 3.0}]
        if {abs($got - $want) > $tol + 1e-9} {
            lappend bad [format "%s %s: tk=%s (%.1f) rtcl=%s, off %.1f > %.1f" \
                $kind $name $expected $want $got [expr {abs($got - $want)}] $tol]
        }
    }
}

puts [format "tk-geometry: checked=%d out-of-tolerance=%d (font scale %s)" \
    $checked [llength $bad] $scale]
foreach line $bad { puts "  $line" }
exit [expr {[llength $bad] > 0}]
