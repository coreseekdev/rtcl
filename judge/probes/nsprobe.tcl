namespace eval n { variable v 5 }
puts "A:[set ::n::v]"
puts "B:[namespace eval n { set ::n::v }]"
namespace eval n { variable w 6 }
namespace delete n
puts "C:[info exists ::n::w]"
namespace eval m { variable v 7 }
puts "D:[namespace which -variable ::m::v]"
puts "E:[namespace which -variable v]"
