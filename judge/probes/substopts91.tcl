# subst 9.1 option face (-backslashes/-commands/-variables) — pins.
# Source: tclCmdMZ.c TclSubstOptions + doc/subst.n.
set x 5
puts [subst -backslashes {a\tb}]
puts [subst -commands {a[expr 1+1]b}]
puts [subst -variables {a$x b}]
puts [subst -commands -variables {a[expr 1+1]c$x}]
catch {subst -backslashes -nocommands {a\tb}} m; puts "1:$m"
catch {subst -foo {x}} m; puts "2:$m :: $errorCode"
catch {subst -commands -backslashes -variables {x}} m; puts "3:$m"
catch {subst foo bar} m; puts "4:$m :: $errorCode"
catch {subst -no bar} m; puts "5:$m :: $errorCode"
catch {subst -bogus bar} m; puts "6:$m :: $errorCode"
catch {subst} m; puts "7:$m"
puts [subst -commands -variables {abc $x [expr {1 + 2}] \\\x41}]
puts [subst -backslashes -variables {abc $x [expr {1 + 2}] \\\x41}]
puts [subst -backslashes -commands {abc $x [expr {1 + 2}] \\\x41}]
puts [subst {a\tb $x [expr 1+1]}]
puts [subst -novariables {a\tb $x}]
puts [subst -nobackslashes -novariables {a\tb $x}]
# exact spelling wins over prefix ambiguity
puts [subst -commands {a[expr 1+1]b}]
puts [subst -variables {a$x}]
