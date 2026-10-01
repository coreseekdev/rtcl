set log {}
proc foo {} {}
trace add command foo rename {append ::log R;#}
rename foo bar
rename bar baz
puts "1: [trace info command baz]"
puts "c=[catch {trace info command foo} m] m=$m"
puts "c2=[catch {trace info command neverexisted} m2] m2=$m2"
