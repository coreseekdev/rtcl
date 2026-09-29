puts "A:[dict map {k v} {a 1} {return -level 0 X}]"
puts "B:[dict map {k v} {a 1 b 2} {return -level 0 "$k,$v"}]"
set r [catch {dict map {k v} {a 1} {return -level 0 X}} m]; puts "C: c=$r m=$m"
puts "D:[dict map {k v} {a 1} {expr {$v+1}}]"
# level-0 return inside plain script context vs proc
proc p {} { return -level 0 Q }; puts "E:[p]"
catch {return -level 0 R} m2; puts "F: m=$m2"
# return INSIDE dict for body
dict for {kk vv} {a 1} { return -level 0 "$kk,$vv" }
puts "G: done"
