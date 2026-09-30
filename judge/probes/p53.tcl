puts [scan {1 2 3} {%e %f %g} x y z]|$x:$y:$z
puts [scan 2 %f y]|$y
puts [scan {13.6} %f y]|$y
puts [scan {123 13.6} {%s %f} a b]|$a:$b
puts [scan {2 3} {%e %f} x y]|$x:$y
puts [scan 13.6 %f]
