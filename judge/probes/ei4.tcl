proc E {label script} {
    catch {uplevel 1 $script} m
    puts "$label<<$::errorInfo>>"
}
E F1 {foreach i {1 2} {boomf}}
E F2 {if 1 {boomi}}
E F3 {eval {boome}}
E F4 {switch 1 a {booms}}
E F5 {apply {{} {boomap}}}
# trailing space / semicolon rule
proc tsp {} { boom-sp }
E F6 {tsp}
proc tsp2 {} { boom-sp2   ; }
E F7 {tsp2}
# nested namespace eval frames
E F8 {namespace eval o1 {namespace eval o2 {boomn}}}
# command subst at top of catch script (no uplevel)
catch {set q [boomq]} m
puts "F9<<$::errorInfo>>"
# error inside catch inner, errorInfo visible INSIDE
catch {
    catch {boom-in} m2
    puts "F10-inside<<$::errorInfo>>"
} m
# source frame check
puts "F11-done"
