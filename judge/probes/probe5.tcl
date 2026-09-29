puts "zwsp: space=[string is space ​] cntrl=[string is control ​] print=[string is print ​] graph=[string is graph ​]"
puts "U+200B: space=[string is space ​] cntrl=[string is control ​]"
puts "nbsp: space=[string is space  ] print=[string is print  ]"
puts "tab: space=[string is space \t] cntrl=[string is control \t] print=[string is print \t] graph=[string is graph \t]"
puts "adlam U+1E950 digit=[string is digit \U0001E950]"
puts "bad class msg:"
catch {string is bogus x} m
puts $m
catch {string is} m2
puts $m2
catch {string} m3
puts $m3
catch {string length} m4
puts $m4
catch {string length a b} m5
puts $m5
catch {string is integer} m6
puts $m6
catch {string is integer -failstrict x} m7
puts $m7
catch {string is integer -strict x extra} m8
puts $m8
