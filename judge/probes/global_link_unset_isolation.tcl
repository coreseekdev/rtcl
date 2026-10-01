set y 99
proc p {} { global y; unset y; }
p
puts "after=[catch {set y} m] <$m> exists=[info exists y]"
set z 7
proc q {} { global z; unset z; return [info exists z] }
puts "q-inside=[q] after-q=[info exists z]"
