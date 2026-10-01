unset -nocomplain x
set x(bar) 0
trace add variable x read {set x(foo) 1 ;#}
trace add variable x read {unset -nocomplain x ;#}
puts "r=[catch {array get x} res] res=$res"
puts "post-exists=[array exists x] post=[array get x]"
# same but swap registration (1.14)
unset -nocomplain x
set x(bar) 0
trace add variable x read {unset -nocomplain x ;#}
trace add variable x read {set x(foo) 1 ;#}
puts "r14=[catch {array get x} res] res=$res"
puts "post14-exists=[array exists x] post14=[array get x]"
# P2 recreation post-state
unset -nocomplain a
set a(bar) 0
trace add variable a read {unset -nocomplain a; set a(bar) 9 ;#}
puts "p2=[catch {array get a} r2] r2=$r2 post2=[array get a]"
