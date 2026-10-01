namespace eval q {variable w:::x 9}
puts "exists-ns-qw=[namespace exists ::q::w]"
puts "c=[catch {set ::q::w::x} r] r=$r"
puts "vars-q=[info vars ::q::*]"
namespace eval q {puts "inside=[info vars]"}
namespace eval q {variable y::z 9}
puts "exists-ns-qy=[namespace exists ::q::y]"
puts "c2=[catch {set ::q::y::z} r2] r2=$r2"
