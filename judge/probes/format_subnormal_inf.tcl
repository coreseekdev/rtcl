puts [format %.17g 1e-321]
puts [format %.17g 1.5e-320]
puts [format %.17g Inf]
puts [format %.17g -Inf]
puts [format %g Inf]
puts [format %.17g 1.1116477031428047e-321]
puts [scan 1.[string repeat 1 30]e-321 %g d]; puts $d
puts [format %.17g $d]
