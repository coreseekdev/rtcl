puts "p512=[catch {set b a$()} msg] <$msg>"
puts "p710=[catch {eval "list a b\\
c d"} m] <$m>"
puts "p711=[catch {eval "list a \"b c\"\\
d e"} m] <$m>"
puts "p152=[info complete "abc\\
"]"
set x 5
puts "p1014=[catch {eval \$x[format "%010d" 0](} msg] <$msg>"
