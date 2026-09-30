foreach {exp numdig} {-321 190 -400 110 -221 290 -121 390 308 202 400 110 221 291 121 391} {
    set s 1.[string repeat 1 $numdig]e$exp
    set d no_scan
    scan $s %g d
    puts "$exp/$numdig -> [format %.17g $d]"
}
