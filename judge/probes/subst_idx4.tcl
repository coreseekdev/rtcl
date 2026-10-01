proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t a {proc tstProc {} {
	global a
	set tst $a([winfo name $zz])
	# this is a bogus comment
	# this is a bogus comment
    }
    set msg {}
    list [catch tstProc msg] $msg}
t b {proc tstProc2 {} {
	global a
	set tst $a([winfo name $zz])
    }
    list [catch tstProc2 m] $m}
t c {proc tstProc3 {} {global a; set tst $a([winfo name $zz]); }; list [catch tstProc3 m] $m}
t d {set a(x) 1; proc tstProc4 {} {global a; set tst $a([winfo name $zz])}; list [catch tstProc4 m] $m}
