# crash-probe: d10 tailcall ns
source [file join [file dirname [info script]] _chk.tcl]
proc p {} { return "global p" }
namespace eval a { proc p {} { return "a::p" } ; proc q {} { tailcall p } }
chk ns [a::q] a::p
set g 0
proc h {} { tailcall set g 1 }
h
chk g $g 1
