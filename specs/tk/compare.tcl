# Compares two runs of specs/tk/font-metrics.tcl: real Tk's and rtcl's.
#
#   tclsh specs/tk/compare.tcl tk.out rtcl.out ?fontScale?
#
# Tk measures in device pixels at Xft's resolution; rtcl lays out in 96 dpi
# logical pixels.  `fontScale` (Xft.dpi / 96, 1 by default) brings Tk's
# numbers to logical pixels first.  What must then agree:
#
#   - `tk scaling`, the empty string's width (0) and `-fixed`: exactly;
#   - widths: within half a pixel per glyph (hinted advances are whole
#     pixels, rounded at each side's own resolution) plus 4 %;
#   - `-linespace`: within 1.5 px (it is what lines are laid out by);
#   - `-ascent`/`-descent`: within 3 px.  The two toolkits read the split
#     of a line from different tables of some faces (Courier here), so only
#     their sum is held tightly.
#
# Prints every line that is out of tolerance and exits 1 if there is one.

lassign $argv tkFile rtclFile scale
if {$rtclFile eq ""} {
    puts stderr "usage: compare.tcl tk.out rtcl.out ?fontScale?"
    exit 2
}
if {$scale eq ""} { set scale 1.0 }

proc readRun {file} {
    set fh [open $file]
    set rows {}
    foreach line [split [read -nonewline $fh] \n] {
        lassign [split $line \t] kind font what value
        dict set rows [list $kind $font $what] $value
    }
    close $fh
    return $rows
}

set tk [readRun $tkFile]
set rtcl [readRun $rtclFile]
set checked 0
set bad {}
set worst 0.0

dict for {key expected} $tk {
    lassign $key kind font what
    if {![dict exists $rtcl $key]} {
        lappend bad "missing in rtcl: $key"
        continue
    }
    set got [dict get $rtcl $key]
    incr checked
    if {$kind eq "S"} {
        if {abs($expected - $got) > 0.001} {
            lappend bad "tk scaling: tk=$expected rtcl=$got"
        }
        continue
    }
    if {$kind eq "X" && $what eq "-fixed"} {
        if {$expected != $got} {
            lappend bad "$font -fixed: tk=$expected rtcl=$got"
        }
        continue
    }
    set want [expr {$expected / double($scale)}]
    if {$kind eq "M"} {
        set glyphs [string length $what]
        set tolerance [expr {$glyphs == 0 ? 0.0 : 0.5 * $glyphs + 0.04 * $want}]
        set label "$font measure \"$what\""
    } elseif {$what eq "-linespace"} {
        set tolerance 1.5
        set label "$font $what"
    } else {
        set tolerance 3.0
        set label "$font $what"
    }
    set off [expr {abs($got - $want)}]
    if {$kind eq "M" && $want > 0} {
        set worst [expr {max($worst, 100.0 * $off / $want)}]
    }
    if {$off > $tolerance + 1e-9} {
        lappend bad [format "%s: tk=%s (%.1f logical) rtcl=%s, off by %.1f > %.1f" \
            $label $expected $want $got $off $tolerance]
    }
}

puts [format "tk-font-metrics: checked=%d out-of-tolerance=%d (font scale %s, widest width deviation %.1f%%)" \
    $checked [llength $bad] $scale $worst]
foreach line $bad { puts "  $line" }
exit [expr {[llength $bad] > 0}]
