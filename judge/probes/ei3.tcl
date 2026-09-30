proc E {label script} {
    catch {uplevel 1 $script} m
    puts "$label<<$::errorInfo>>"
}
# 1: nested proc frames
proc lvl2 {} { boom }
proc lvl1 {} { lvl2 }
E N1 {lvl1}
# 2: multi-line ns eval body, error on line 3
catch {namespace eval nl {
    set a 1
    set b 2
    boom-here
}} m
puts "N2<<$::errorInfo>>"
# 3: ns eval command itself mid-file (line of the ns eval cmd?)
set x 1
set y 2
catch {namespace eval n3 {boom3}} m
puts "N3<<$::errorInfo>>"
# 4: error with info inside proc
proc ei {} { error msg1 info1 }
E N4 {ei}
# 5: error without info inside proc
proc ei2 {} { error msg2 }
E N5 {ei2}
# 6: second error overwrites?
catch {first-error} m
catch {second-error} m
puts "N6<<$::errorInfo>>"
# 7: command substitution frame
E N7 {set v [boomsub]}
# 8: while body error
E N8 {while 1 {boomw}}
# 9: expr error frames
E N9 {expr {1 + }}
# 10: unknown command vs error cmd frames (bare, at catch level)
catch {bare-boom} m
puts "N10<<$::errorInfo>>"
