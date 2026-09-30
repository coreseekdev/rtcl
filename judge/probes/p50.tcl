catch {expr {123456789012345678901234567890*$foo(["abcdefghijklmnopqrstuvwxyz])}} m
puts "R<<$m>>"
catch {expr {123456789012345678901234567890*$bar(abcdefghijklmnopqrstuvwxyz}} m2
puts "R52<<$m2>>"
catch {expr {123456789012345678901234567890*$bar([""abcdefghijklmnopqrstuvwxyz])}} m3
puts "R54<<$m3>>"
