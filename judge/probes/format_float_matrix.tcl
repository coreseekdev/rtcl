foreach spec {%f %e %g %E %G %.0f %.0e %.0g %10g %-10g| %010g %.17g %20.10f %#g %#.3g %+.2f %.3e} {
    foreach v {Inf -Inf NaN 0.0 1.5 123456.789 0.000123456 1e20 123456789 1234567.0 123456.5 0.5} {
        if {[catch {format $spec $v} r]} { puts "ERR $spec $v: $r" } else { puts "$spec|$v -> $r" }
    }
}
puts [format %g 100000.0]
puts [format %g 1000000.0]
puts [format %g 0.0001]
puts [format %g 0.00001]
puts [format %.15g 0.1]
puts [format %.1g 0.0]
puts [format %#.0g 1.0]
puts [format %.17g 0.1]
puts [format %e 0.0]
puts [format %.2e -12345.678]
