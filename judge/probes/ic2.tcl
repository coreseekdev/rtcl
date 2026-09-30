proc gp {} {}
namespace eval e1 {
  proc c1 {} {}
  namespace eval deep { proc d {} {} }
  puts "in-e1-all: [info commands ::*]"
  puts "in-e1-gp: [info commands gp]"
  puts "in-deep-all: [info commands ::*]"
  puts "in-deep-gp: [info commands gp]"
}
puts "top: [info commands ::e1::*]"
namespace eval e2 { namespace export *; proc z {} {}; namespace ensemble create }
puts "ens: [info commands ::e2*]"
puts "ens2: [info commands e2]"
namespace eval e3 { proc w {} {}; namespace ensemble create -command ::myens }
puts "ens3: [info commands myens]"
