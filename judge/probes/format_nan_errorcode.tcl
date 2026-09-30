puts [catch {format %f NaN} r]; puts $r
set code [catch {format %f NaN} m opt]; puts [dict get $opt -errorcode]
set code [catch {format %f abc} m opt]; puts "$m | [dict get $opt -errorcode]"
puts [format %f -0.0]
puts [format %.17g -0.0]
puts [format %e -0.0]
puts [format %g 1.1116477031428047e-321]
puts [format %.17g 1e-321]
puts [format %.4g 999950]
puts [format %.4g 999960]
puts [format %g 123456]
