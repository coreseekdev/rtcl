namespace eval p1 {}
proc p1::sub {} {}
puts "A:[info commands ::p1*]"
puts "B:[info commands ::p1::*]"
puts "C:[info commands ::p1]"
puts "D:[info procs ::p1*]"
namespace eval q1 { proc zz {} {} }
puts "G:[info commands ::q1*]"
puts "H:[info commands q1*]"
puts "I:[info commands ::q1::zz]"
puts "J:[info commands ::q1::z*]"
puts "K:[info commands ::bogus::*]"
puts "L:[info commands ::p1]"
