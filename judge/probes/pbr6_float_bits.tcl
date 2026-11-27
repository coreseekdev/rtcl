# pbr6_float_bits — binary format/scan IEEE-754 bit fidelity for the
# non-finite and signed-zero values (tclsh 9.1.0).
#
# R/r are single precision (big/little endian), Q/q double precision.
# 9.1.0 emits the canonical IEEE bit patterns:
#   float32: Inf 7f800000, -Inf ff800000, NaN 7fc00000, -0.0 80000000
#   float64: Inf 7ff0000000000000, -Inf fff0000000000000,
#            NaN 7ff8000000000000, -0.0 8000000000000000
# (rtcl used to saturate to 7f7fffff / 7fefffffffffffff; the spec now
# anchors the oracle bytes.)

foreach {fmt val} {
    R Inf   R -Inf   R NaN   R -0.0
    r Inf   r -Inf   r NaN   r -0.0
    Q Inf   Q -Inf   Q NaN   Q -0.0
    q Inf   q -Inf   q NaN   q -0.0
    R 1.0   r 1.0    Q 1.0   q 1.0
} {
    puts "$fmt $val [binary encode hex [binary format $fmt $val]]"
}

# round-trip: scan must recover the same non-finite value
foreach {fmt val} {R Inf R -Inf Q Inf Q -Inf} {
    binary scan [binary format $fmt $val] $fmt back
    puts "rt $fmt $val $back"
}
