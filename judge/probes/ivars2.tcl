set g 9
namespace eval n { variable v 1 }
puts "A:[lsort [info vars]]"
puts "B:[lsort [info globals]]"
