catch {return {x}; bogus code} m
puts "c1: $m"
set m2 [catch {
  set a 1
  return 5
  set a 2
} m3]
puts "c2: c=$m2 r=$m3"
