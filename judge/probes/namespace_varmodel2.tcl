set v global-v
namespace eval n { variable v ns-v
    set ::r1 [set v]
    set ::r2 [info vars]
}
puts "R1=$r1 VARS=$r2"
namespace eval n { set ::r4 [info exists v] }
puts "R4=$r4"
namespace eval deep::sub {}
namespace eval deep::sub { variable w sw; set ::r5 [set w] }
puts "R5=$r5"
