namespace eval e1 { proc c1 {} {}
  namespace eval deep {
    puts "REL:[info commands e1::c1]"
    puts "RELSTAR:[info commands e1::*]"
  }
}
namespace eval e1::deep { puts "ABS:[info commands ::e1::c1]" }
