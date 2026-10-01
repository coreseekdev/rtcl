puts "u1=[catch {::tcl::unsupported::assemble} m] <$m>"
puts "u2=[catch {::tcl::unsupported::assemble { frobnicate }} m] <$m>"
puts "u3=[catch {::tcl::unsupported::assemble {push}} m] <$m>"
puts "r1=[::tcl::unsupported::assemble {push hello}]"
puts "r2=[catch {::tcl::unsupported::assemble {push a; push b}} m2] <$m2>"
puts "r3=[::tcl::unsupported::assemble {
    push set
    push x
    push 42
    invokeStk 3
    pop
}]"
puts "r4=[catch {::tcl::unsupported::assemble {invokeStk 2}} m3] <$m3>"
